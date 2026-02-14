use actix_web::{web, HttpResponse, Responder};

use crate::protocol::{AgentRequest, ClarifyAnswer};
use crate::web_ui::{
    merge_shared_config_into_draft, normalize_resume_draft_defaults, read_global_history_limits,
    read_project_config, read_resume_info, read_ui_cache, start_from_payload, AppState,
    ProjectConfigQuery, PushPayload, ResumePayload, StartPayload, UiStateResponse,
};

pub(crate) async fn start_session(
    data: web::Data<AppState>,
    body: web::Json<StartPayload>,
) -> impl Responder {
    let payload = body.into_inner();
    let (history_max_messages, history_max_chars) = {
        let _guard = data.projects_lock.lock().unwrap();
        let cache = read_ui_cache();
        read_global_history_limits(cache.global_options.as_ref())
    };
    match start_from_payload(
        &data,
        payload,
        &history_max_messages,
        &history_max_chars,
        false,
    ) {
        Ok(_) => HttpResponse::Ok().body("started"),
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

pub(crate) async fn get_project_config(query: web::Query<ProjectConfigQuery>) -> impl Responder {
    match read_project_config(&query.workspace) {
        Some(cfg) => HttpResponse::Ok().json(cfg),
        None => HttpResponse::NotFound().body("project config not found"),
    }
}

pub(crate) async fn resume_session(
    data: web::Data<AppState>,
    body: web::Json<ResumePayload>,
) -> impl Responder {
    let payload = body.into_inner();
    if payload.project_id.trim().is_empty() {
        return HttpResponse::BadRequest().body("project_id is empty");
    }
    let (mut draft, history_max_messages, history_max_chars) = {
        let _guard = data.projects_lock.lock().unwrap();
        let cache = read_ui_cache();
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
    let resume = read_resume_info(&draft.workspace, &draft.goal);
    if resume.as_ref().map(|x| x.resumable).unwrap_or(false) {
        match start_from_payload(
            &data,
            draft.into_start(),
            &history_max_messages,
            &history_max_chars,
            true,
        ) {
            Ok(_) => HttpResponse::Ok().body("resumed"),
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

pub(crate) async fn get_ui_state(data: web::Data<AppState>) -> impl Responder {
    let runtime = data.runtime.lock().unwrap().clone();
    let resume = if runtime.last_workspace.trim().is_empty() {
        None
    } else {
        read_resume_info(&runtime.last_workspace, &runtime.last_goal)
    };
    HttpResponse::Ok().json(UiStateResponse { runtime, resume })
}

pub(crate) async fn answer_clarify(
    data: web::Data<AppState>,
    body: web::Json<crate::web_ui_models::ClarifyPayload>,
) -> impl Responder {
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
    HttpResponse::Ok().body("clarify sent")
}

pub(crate) async fn push_remote(
    data: web::Data<AppState>,
    body: web::Json<PushPayload>,
) -> impl Responder {
    let payload = body.into_inner();
    let req = AgentRequest::PushRemote {
        remote: payload.remote,
        url: payload.url,
        branch: payload.branch,
    };
    if let Err(e) = data.tx_req.send(req) {
        return HttpResponse::InternalServerError().body(format!("send push failed: {}", e));
    }
    HttpResponse::Ok().body("push sent")
}

pub(crate) async fn revert_last(data: web::Data<AppState>) -> impl Responder {
    if let Err(e) = data.tx_req.send(AgentRequest::RevertLast) {
        return HttpResponse::InternalServerError().body(format!("send revert failed: {}", e));
    }
    HttpResponse::Ok().body("revert sent")
}

pub(crate) async fn stop_session(data: web::Data<AppState>) -> impl Responder {
    let mut runtime = data.runtime.lock().unwrap();
    runtime.running = false;
    data.stop_now
        .store(true, std::sync::atomic::Ordering::Relaxed);
    match data.tx_req.send(AgentRequest::Stop) {
        Ok(_) => HttpResponse::Ok().body("stop sent"),
        Err(e) => {
            runtime.last_error = format!("stop channel unavailable: {}", e);
            HttpResponse::Ok().body("stop acknowledged (channel unavailable)")
        }
    }
}
