use actix_web::{web, HttpRequest, HttpResponse, Responder};

use crate::lock_utils::lock_recover;
use crate::protocol::{AgentRequest, ClarifyAnswer};
use crate::web_ui::{
    merge_shared_config_into_draft, normalize_resume_draft_defaults, read_global_history_limits,
    read_project_config_for_root, read_resume_info_for_root, read_ui_cache_for_root,
    start_from_payload, AppState, ProjectConfigQuery, PushPayload, ResumePayload, StartPayload,
    UiStateResponse,
};
use crate::web_ui_authz;

pub(crate) async fn start_session(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<StartPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    if let Err(e) = web_ui_authz::ensure_user_dirs(&ctx) {
        return HttpResponse::InternalServerError().body(format!("init user dirs failed: {}", e));
    }
    let payload = body.into_inner();
    let workspace_for_audit = payload.workspace.clone();
    let (history_max_messages, history_max_chars) = {
        let _guard = lock_recover(&data.projects_lock, "projects_lock");
        let cache = read_ui_cache_for_root(&ctx.managed_root_dir);
        read_global_history_limits(cache.global_options.as_ref())
    };
    match start_from_payload(
        &data,
        payload,
        &ctx.managed_root_dir,
        &history_max_messages,
        &history_max_chars,
        false,
    ) {
        Ok(_) => {
            let mut fields: std::collections::HashMap<&str, String> =
                std::collections::HashMap::new();
            fields.insert("user", ctx.username.clone().unwrap_or_default());
            fields.insert("role", format!("{:?}", ctx.role));
            fields.insert("workspace", workspace_for_audit);
            data.auth.audit("web_start_session", &fields);
            HttpResponse::Ok().body("started")
        }
        Err(e) => {
            if e.contains("api_key is empty") {
                HttpResponse::BadRequest().body(e)
            } else if e.contains("already running") {
                HttpResponse::Conflict().body(e)
            } else {
                HttpResponse::InternalServerError().body(e)
            }
        }
    }
}

pub(crate) async fn get_project_config(
    req: HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<ProjectConfigQuery>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    match read_project_config_for_root(&query.workspace, &ctx.managed_root_dir) {
        Some(cfg) => HttpResponse::Ok().json(cfg),
        None => HttpResponse::NotFound().body("project config not found"),
    }
}

pub(crate) async fn resume_session(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<ResumePayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    if let Err(e) = web_ui_authz::ensure_user_dirs(&ctx) {
        return HttpResponse::InternalServerError().body(format!("init user dirs failed: {}", e));
    }
    let payload = body.into_inner();
    if payload.project_id.trim().is_empty() {
        return HttpResponse::BadRequest().body("project_id is empty");
    }
    let (mut draft, history_max_messages, history_max_chars) = {
        let _guard = lock_recover(&data.projects_lock, "projects_lock");
        let cache = read_ui_cache_for_root(&ctx.managed_root_dir);
        let (history_max_messages, history_max_chars) =
            read_global_history_limits(cache.global_options.as_ref());
        let Some(project) = cache
            .projects
            .into_iter()
            .find(|p| p.id == payload.project_id)
        else {
            return HttpResponse::NotFound().body("project not found");
        };
        let mut snapshot = project.snapshot;
        if let Some(cfg) = cache.shared_config.as_ref() {
            merge_shared_config_into_draft(&mut snapshot, cfg);
        }
        (snapshot, history_max_messages, history_max_chars)
    };
    normalize_resume_draft_defaults(&mut draft);
    draft.unattended_mode = payload.unattended_mode;
    if draft.api_key.trim().is_empty() {
        return HttpResponse::BadRequest().body("api_key is empty in snapshot");
    }
    let resume = read_resume_info_for_root(&draft.workspace, &ctx.managed_root_dir, &draft.goal);
    if resume.as_ref().map(|x| x.resumable).unwrap_or(false) {
        match start_from_payload(
            &data,
            draft.into_start(),
            &ctx.managed_root_dir,
            &history_max_messages,
            &history_max_chars,
            true,
        ) {
            Ok(_) => {
                let mut fields: std::collections::HashMap<&str, String> =
                    std::collections::HashMap::new();
                fields.insert("user", ctx.username.clone().unwrap_or_default());
                fields.insert("role", format!("{:?}", ctx.role));
                fields.insert("project_id", payload.project_id);
                data.auth.audit("web_resume_session", &fields);
                HttpResponse::Ok().body("resumed")
            }
            Err(e) => {
                if e.contains("already running") {
                    HttpResponse::Conflict().body(e)
                } else {
                    HttpResponse::InternalServerError().body(e)
                }
            }
        }
    } else {
        HttpResponse::BadRequest().body("no matching resumable session")
    }
}

pub(crate) async fn get_ui_state(req: HttpRequest, data: web::Data<AppState>) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let runtime = lock_recover(&data.runtime, "runtime").clone();
    let resume = if runtime.last_workspace.trim().is_empty() {
        None
    } else {
        read_resume_info_for_root(
            &runtime.last_workspace,
            &ctx.managed_root_dir,
            &runtime.last_goal,
        )
    };
    HttpResponse::Ok().json(UiStateResponse { runtime, resume })
}

pub(crate) async fn answer_clarify(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<crate::web_ui_models::ClarifyPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let mut answers_vec = Vec::new();
    if let Some(map) = body.answers.as_object() {
        for (id, value) in map {
            let (qtype, single, multi, text) = match value {
                serde_json::Value::String(s) => ("single", s.clone(), Vec::new(), String::new()),
                serde_json::Value::Array(arr) => {
                    let multi = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect::<Vec<_>>();
                    ("multi", String::new(), multi, String::new())
                }
                _ => ("text", String::new(), Vec::new(), value.to_string()),
            };
            answers_vec.push(ClarifyAnswer {
                id: id.to_string(),
                qtype: qtype.to_string(),
                single,
                multi,
                text,
            });
        }
    }
    if let Err(e) = data.tx_req.send(AgentRequest::Clarify {
        answers: answers_vec,
    }) {
        return HttpResponse::InternalServerError().body(format!("send clarify failed: {}", e));
    }
    {
        let mut fields: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
        fields.insert("user", ctx.username.clone().unwrap_or_default());
        fields.insert("role", format!("{:?}", ctx.role));
        data.auth.audit("web_clarify", &fields);
    }
    HttpResponse::Ok().body("clarify sent")
}

pub(crate) async fn push_remote(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<PushPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let payload = body.into_inner();
    let req = AgentRequest::PushRemote {
        remote: payload.remote,
        url: payload.url,
        branch: payload.branch,
    };
    if let Err(e) = data.tx_req.send(req) {
        return HttpResponse::InternalServerError().body(format!("send push failed: {}", e));
    }
    {
        let mut fields: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
        fields.insert("user", ctx.username.clone().unwrap_or_default());
        fields.insert("role", format!("{:?}", ctx.role));
        data.auth.audit("web_push_remote", &fields);
    }
    HttpResponse::Ok().body("push sent")
}

pub(crate) async fn revert_last(req: HttpRequest, data: web::Data<AppState>) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    if let Err(e) = data.tx_req.send(AgentRequest::RevertLast) {
        return HttpResponse::InternalServerError().body(format!("send revert failed: {}", e));
    }
    {
        let mut fields: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
        fields.insert("user", ctx.username.clone().unwrap_or_default());
        fields.insert("role", format!("{:?}", ctx.role));
        data.auth.audit("web_revert_last", &fields);
    }
    HttpResponse::Ok().body("revert sent")
}

pub(crate) async fn stop_session(req: HttpRequest, data: web::Data<AppState>) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let mut runtime = lock_recover(&data.runtime, "runtime");
    runtime.running = false;
    data.stop_now
        .store(true, std::sync::atomic::Ordering::Relaxed);
    match data.tx_req.send(AgentRequest::Stop) {
        Ok(_) => {
            let mut fields: std::collections::HashMap<&str, String> =
                std::collections::HashMap::new();
            fields.insert("user", ctx.username.clone().unwrap_or_default());
            fields.insert("role", format!("{:?}", ctx.role));
            data.auth.audit("web_stop_session", &fields);
            HttpResponse::Ok().body("stop sent")
        }
        Err(e) => {
            runtime.last_error = format!("stop channel unavailable: {}", e);
            HttpResponse::Ok().body("stop acknowledged (channel unavailable)")
        }
    }
}
