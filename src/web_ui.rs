//! Minimal web UI mode for the KACF (Kinogram AutoCoding Framework) agent.
//!
//! This module exposes HTTP endpoints to control the agent loop and to
//! retrieve events (logs, diffs, clarifications, completion). It is
//! intended to allow running the system on machines without a native
//! desktop environment (e.g. Termux) and interacting through a
//! browser. The server listens on 0.0.0.0:8080 by default.

use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use async_stream::stream;
use crossbeam_channel::{Receiver, Sender};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::auth;
use crate::deepseek_api;
use crate::lock_utils::lock_recover;
use crate::protocol::{AgentEvent, AgentRequest};
use crate::web_ui_analytics;
use crate::web_ui_authz;
use crate::web_ui_cache_logic;
use crate::web_ui_debug;
use crate::web_ui_events::{self, EventBuffer, SerializableEvent};
use crate::web_ui_languages;
use crate::web_ui_models;
pub(crate) use crate::web_ui_models::{
    GlobalOptions, HealthResponse, LanguageListResponse, MetricsResponse, ProjectConfig,
    ProjectConfigQuery, PushPayload, ResumeInfo, ResumeMeta, ResumePayload, RuntimeStatus,
    SharedConfig, SlugSuggestPayload, SlugSuggestResponse, StartPayload, UiCachePatch,
    UiCachePayload, UiStateResponse, WebProject,
};
use crate::web_ui_projects;
use crate::web_ui_runtime_env;
use crate::web_ui_runtime_metrics;
use crate::web_ui_session;
use crate::web_ui_slug;
use crate::web_ui_store;
use crate::web_ui_vm;

/// Index HTML page embedded at compile time.
const INDEX_HTML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/static/index.html"));
const DIFF_HTML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/static/diff.html"));
const APP_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/static/app.css"));
const DIFF_JS: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/static/js/diff_view.js"));
const APP_JS: &str = concat!(
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/static/js/app_state.js"
    )),
    "\n",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/static/js/app_auth.js"
    )),
    "\n",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/static/js/app_ui_cache.js"
    )),
    "\n",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/static/js/app_log_pipeline.js"
    )),
    "\n",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/static/js/app_runtime_state.js"
    )),
    "\n",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/static/js/app_projects.js"
    )),
    "\n",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/static/js/app_runtime_sync.js"
    )),
    "\n",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/static/js/app_vm.js"
    )),
    "\n",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/static/js/app_runtime_actions.js"
    ))
);

const SESSION_STATE_FILENAME: &str = ".autocoding_state.json";
const PROJECT_CONFIG_FILENAME: &str = ".autocoding_project.json";
const UI_CACHE_FILENAME: &str = ".autocoding_webui_cache.json";
const MANAGED_ROOT_DIR: &str = "autocoding_data";
const MANAGED_WORKSPACES_DIR: &str = "workspaces";

// User-friendly defaults: keep these fixed (no UI inputs).
const FIXED_RELEASE_GATE_THRESHOLD: u8 = 75;

#[derive(Clone)]
pub struct AppState {
    pub(crate) tx_req: Sender<AgentRequest>,
    pub(crate) events: Arc<Mutex<EventBuffer>>,
    pub(crate) event_bytes: Arc<Mutex<usize>>,
    pub(crate) next_event_id: Arc<Mutex<usize>>,
    pub(crate) runtime: Arc<Mutex<RuntimeStatus>>,
    pub(crate) projects_lock: Arc<Mutex<()>>,
    pub(crate) debug_client_logs: web_ui_debug::DebugLogStore,
    pub(crate) stop_now: Arc<AtomicBool>,
    pub(crate) auth: auth::AuthStore,
}

type DraftPayload = web_ui_models::DraftPayload;

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn managed_root_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(MANAGED_ROOT_DIR)
}

pub(crate) fn merge_shared_config_into_draft(draft: &mut DraftPayload, cfg: &SharedConfig) {
    if !cfg.api_key.trim().is_empty() {
        draft.api_key = cfg.api_key.trim().to_string();
    }
    if !cfg.base_url.trim().is_empty() {
        draft.base_url = cfg.base_url.trim().to_string();
    }
    if !cfg.model.trim().is_empty() {
        draft.model = cfg.model.trim().to_string();
    }
    if !cfg.language.trim().is_empty() {
        draft.language = cfg.language.trim().to_string();
    }
}

pub(crate) fn normalize_resume_draft_defaults(draft: &mut DraftPayload) {
    if draft.base_url.trim().is_empty() {
        draft.base_url = web_ui_models::default_base_url();
    }
    if draft.model.trim().is_empty() {
        draft.model = web_ui_models::default_model_name();
    }
}

fn require_managed_workspace_for_root(
    workspace: &str,
    managed_root_dir: &str,
) -> Result<PathBuf, String> {
    web_ui_store::require_managed_workspace(workspace, managed_root_dir, MANAGED_WORKSPACES_DIR)
}

fn save_project_config_for_root(
    workspace: &str,
    managed_root_dir: &str,
    unattended_mode: bool,
) -> std::io::Result<()> {
    let cfg = ProjectConfig {
        unattended_mode,
        updated_at_unix: now_unix(),
    };
    web_ui_store::save_project_config(
        workspace,
        managed_root_dir,
        MANAGED_WORKSPACES_DIR,
        PROJECT_CONFIG_FILENAME,
        &cfg,
    )
}

pub(crate) fn read_project_config_for_root(
    workspace: &str,
    managed_root_dir: &str,
) -> Option<ProjectConfig> {
    web_ui_store::read_project_config(
        workspace,
        managed_root_dir,
        MANAGED_WORKSPACES_DIR,
        PROJECT_CONFIG_FILENAME,
    )
}

fn sanitize_language_code(raw: &str) -> Option<String> {
    web_ui_languages::sanitize_language_code(raw)
}

fn list_language_packs() -> Result<Vec<String>, String> {
    web_ui_languages::list_language_packs()
}

fn read_language_pack(code: &str) -> Result<String, String> {
    web_ui_languages::read_language_pack(code)
}

fn load_language_packs_checked() -> Result<(), String> {
    web_ui_languages::ensure_language_packs_checked()
}

pub(crate) fn read_ui_cache_for_root(managed_root_dir: &str) -> UiCachePayload {
    web_ui_store::read_ui_cache(managed_root_dir, UI_CACHE_FILENAME)
}

fn write_ui_cache_for_root(
    managed_root_dir: &str,
    payload: &UiCachePayload,
) -> std::io::Result<()> {
    web_ui_store::write_ui_cache(managed_root_dir, UI_CACHE_FILENAME, payload)
}

pub(crate) fn read_global_history_limits(opts: Option<&GlobalOptions>) -> (String, String) {
    let Some(opts) = opts else {
        return (String::new(), String::new());
    };
    (
        opts.history_max_messages.trim().to_string(),
        opts.history_max_chars.trim().to_string(),
    )
}

pub(crate) fn read_resume_info_for_root(
    workspace: &str,
    managed_root_dir: &str,
    goal: &str,
) -> Option<ResumeInfo> {
    let path = require_managed_workspace_for_root(workspace, managed_root_dir)
        .ok()?
        .join(SESSION_STATE_FILENAME);
    let content = fs::read_to_string(path).ok()?;
    let meta: ResumeMeta = serde_json::from_str(&content).ok()?;
    if !goal.trim().is_empty() && !meta.goal.trim().is_empty() && meta.goal.trim() != goal.trim() {
        return Some(ResumeInfo {
            resumable: false,
            updated_at_unix: meta.updated_at_unix,
            iteration: meta.iteration,
            message_count: meta.message_count,
            last_status: Some("发现历史断点，但目标与当前输入不一致".to_string()),
        });
    }
    Some(ResumeInfo {
        resumable: true,
        updated_at_unix: meta.updated_at_unix,
        iteration: meta.iteration,
        message_count: meta.message_count,
        last_status: meta.last_status,
    })
}

pub(crate) fn start_from_payload(
    data: &web::Data<AppState>,
    payload: StartPayload,
    managed_root_dir: &str,
    global_history_max_messages: &str,
    global_history_max_chars: &str,
    resume_from_checkpoint: bool,
) -> Result<(), String> {
    // Clear any previous stop request so the new run can proceed.
    data.stop_now
        .store(false, std::sync::atomic::Ordering::Relaxed);
    if payload.api_key.trim().is_empty() {
        return Err("api_key is empty".to_string());
    }
    if payload.git_user_name.trim().is_empty() || payload.git_user_email.trim().is_empty() {
        return Err("git user identity is empty".to_string());
    }
    {
        let runtime = lock_recover(&data.runtime, "runtime");
        if runtime.running {
            return Err("session already running".to_string());
        }
    }
    let workspace_full = require_managed_workspace_for_root(&payload.workspace, managed_root_dir)?;
    web_ui_runtime_env::apply_runtime_config_envs(
        global_history_max_messages,
        global_history_max_chars,
    );
    let req = AgentRequest::Start {
        api_key: payload.api_key,
        base_url: payload.base_url,
        model: payload.model,
        language: payload.language,
        unattended_mode: payload.unattended_mode,
        resume_from_checkpoint,
        workspace: workspace_full.clone(),
        goal: payload.goal.clone(),
        git_user_name: payload.git_user_name,
        git_user_email: payload.git_user_email,
    };
    if let Err(e) = save_project_config_for_root(
        &payload.workspace,
        managed_root_dir,
        payload.unattended_mode,
    ) {
        eprintln!("save project config failed: {}", e);
    }
    data.tx_req
        .send(req)
        .map_err(|e| format!("send start failed: {}", e))?;
    let mut runtime = lock_recover(&data.runtime, "runtime");
    runtime.running = true;
    runtime.last_start_unix = now_unix();
    runtime.last_workspace = payload.workspace;
    runtime.last_goal = payload.goal;
    runtime.last_error.clear();
    Ok(())
}

async fn list_projects(req: HttpRequest, data: web::Data<AppState>) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if let Err(e) = web_ui_authz::ensure_user_dirs(&ctx) {
        return HttpResponse::InternalServerError().body(format!("init user dirs failed: {}", e));
    }
    {
        let mut fields: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
        fields.insert(
            "user",
            ctx.username.clone().unwrap_or_else(|| "guest".to_string()),
        );
        fields.insert("role", format!("{:?}", ctx.role));
        data.auth.audit("web_list_projects", &fields);
    }
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut items = read_ui_cache_for_root(&ctx.managed_root_dir).projects;
    web_ui_projects::sort_projects_by_updated_desc(&mut items);
    HttpResponse::Ok().json(items)
}

async fn upsert_project(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<WebProject>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        let mut fields: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
        fields.insert(
            "user",
            ctx.username.clone().unwrap_or_else(|| "guest".to_string()),
        );
        fields.insert("role", format!("{:?}", ctx.role));
        data.auth
            .audit("web_upsert_project_denied_readonly", &fields);
        return HttpResponse::Forbidden().body("read-only session");
    }
    if let Err(e) = web_ui_authz::ensure_user_dirs(&ctx) {
        return HttpResponse::InternalServerError().body(format!("init user dirs failed: {}", e));
    }
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let item = match web_ui_projects::normalize_project_for_upsert(body.into_inner(), now_unix()) {
        Ok(v) => v,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };
    let mut cache = read_ui_cache_for_root(&ctx.managed_root_dir);
    web_ui_projects::upsert_project_in_cache(&mut cache, item.clone());
    if let Err(e) = write_ui_cache_for_root(&ctx.managed_root_dir, &cache) {
        return HttpResponse::InternalServerError().body(format!("write projects failed: {}", e));
    }
    {
        let mut fields: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
        fields.insert("user", ctx.username.clone().unwrap_or_default());
        fields.insert("role", format!("{:?}", ctx.role));
        fields.insert("project_id", item.id.clone());
        fields.insert("workspace", item.workspace.clone());
        data.auth.audit("web_upsert_project", &fields);
    }
    HttpResponse::Ok().json(item)
}

async fn delete_project(
    req: HttpRequest,
    data: web::Data<AppState>,
    path: web::Path<String>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        let mut fields: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
        fields.insert(
            "user",
            ctx.username.clone().unwrap_or_else(|| "guest".to_string()),
        );
        fields.insert("role", format!("{:?}", ctx.role));
        data.auth
            .audit("web_delete_project_denied_readonly", &fields);
        return HttpResponse::Forbidden().body("read-only session");
    }
    if let Err(e) = web_ui_authz::ensure_user_dirs(&ctx) {
        return HttpResponse::InternalServerError().body(format!("init user dirs failed: {}", e));
    }
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let id = path.into_inner();
    let mut cache = read_ui_cache_for_root(&ctx.managed_root_dir);
    if let Err(e) = web_ui_projects::delete_project_from_cache(&mut cache, &id) {
        return if e == "project id is empty" {
            HttpResponse::BadRequest().body(e)
        } else {
            HttpResponse::NotFound().body(e)
        };
    }
    if let Err(e) = write_ui_cache_for_root(&ctx.managed_root_dir, &cache) {
        return HttpResponse::InternalServerError().body(format!("write projects failed: {}", e));
    }
    {
        let mut fields: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
        fields.insert("user", ctx.username.clone().unwrap_or_default());
        fields.insert("role", format!("{:?}", ctx.role));
        fields.insert("project_id", id);
        data.auth.audit("web_delete_project", &fields);
    }
    HttpResponse::Ok().body("deleted")
}

async fn get_ui_cache(req: HttpRequest, data: web::Data<AppState>) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    {
        let mut fields: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
        fields.insert(
            "user",
            ctx.username.clone().unwrap_or_else(|| "guest".to_string()),
        );
        fields.insert("role", format!("{:?}", ctx.role));
        data.auth.audit("web_get_ui_cache", &fields);
    }
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let payload = read_ui_cache_for_root(&ctx.managed_root_dir);
    HttpResponse::Ok().json(payload)
}

async fn put_ui_cache(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<UiCachePatch>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        let mut fields: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
        fields.insert(
            "user",
            ctx.username.clone().unwrap_or_else(|| "guest".to_string()),
        );
        fields.insert("role", format!("{:?}", ctx.role));
        data.auth.audit("web_put_ui_cache_denied_readonly", &fields);
        return HttpResponse::Forbidden().body("read-only session");
    }
    if let Err(e) = web_ui_authz::ensure_user_dirs(&ctx) {
        return HttpResponse::InternalServerError().body(format!("init user dirs failed: {}", e));
    }
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let patch = body.into_inner();
    let mut payload = read_ui_cache_for_root(&ctx.managed_root_dir);
    web_ui_cache_logic::apply_ui_cache_patch(&mut payload, patch);
    match write_ui_cache_for_root(&ctx.managed_root_dir, &payload) {
        Ok(_) => {
            let mut fields: std::collections::HashMap<&str, String> =
                std::collections::HashMap::new();
            fields.insert("user", ctx.username.clone().unwrap_or_default());
            fields.insert("role", format!("{:?}", ctx.role));
            data.auth.audit("web_put_ui_cache", &fields);
            HttpResponse::Ok().body("saved")
        }
        Err(e) => HttpResponse::InternalServerError().body(format!("write ui cache failed: {}", e)),
    }
}

async fn suggest_project_slug(body: web::Json<SlugSuggestPayload>) -> impl Responder {
    let payload = body.into_inner();
    let fallback = web_ui_slug::fallback_slug(&payload.project_name, &payload.goal, now_unix());
    if payload.api_key.trim().is_empty() {
        return HttpResponse::Ok().json(SlugSuggestResponse {
            slug: fallback,
            source: "fallback".to_string(),
        });
    }
    let messages = vec![
        deepseek_api::ChatMessage::system(
            "You output a single English kebab-case project slug only. lowercase letters, numbers and hyphen only. max 36 chars."
                .to_string(),
        ),
        deepseek_api::ChatMessage::user(format!(
            "Project name: {}\nProject goal: {}\nOutput slug only.",
            payload.project_name, payload.goal
        )),
    ];
    let mut sink = |_delta: &str| {};
    let slug = match deepseek_api::chat_complete_streaming(
        &payload.base_url,
        &payload.api_key,
        &payload.model,
        &messages,
        &mut sink,
    )
    .await
    {
        Ok(text) => {
            let s = web_ui_slug::normalize_slug(text.trim());
            if s.is_empty() {
                fallback.clone()
            } else {
                s
            }
        }
        Err(_) => fallback.clone(),
    };
    HttpResponse::Ok().json(SlugSuggestResponse {
        slug,
        source: "ai".to_string(),
    })
}

async fn health() -> impl Responder {
    HttpResponse::Ok().json(HealthResponse {
        ok: true,
        service: "kacf-web-ui",
        version: env!("CARGO_PKG_VERSION"),
        unix_time: now_unix(),
    })
}

async fn metrics(data: web::Data<AppState>) -> impl Responder {
    let runtime = lock_recover(&data.runtime, "runtime").clone();
    let events_len = lock_recover(&data.events, "events").len();
    let next_id = *lock_recover(&data.next_event_id, "next_event_id");
    let body =
        web_ui_runtime_metrics::build_metrics_response(&runtime, events_len, next_id, now_unix());
    HttpResponse::Ok().json(body)
}

async fn get_events(
    data: web::Data<AppState>,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> impl Responder {
    let from_id: usize = query.get("from").and_then(|v| v.parse().ok()).unwrap_or(0);
    let events = lock_recover(&data.events, "events");
    let list = web_ui_events::pull_events(&events, from_id, web_ui_events::MAX_EVENTS_PER_PULL);
    HttpResponse::Ok().json(list)
}

async fn stream_events(
    req: HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> impl Responder {
    let from_query = query.get("from").and_then(|v| v.parse().ok()).unwrap_or(0);
    let from_header = req
        .headers()
        .get("Last-Event-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .map(|id| id.saturating_add(1))
        .unwrap_or(0);
    let from_id: usize = from_query.max(from_header);
    let mut next_id = from_id;
    let s = stream! {
        yield Ok::<_, actix_web::Error>(web::Bytes::from_static(b"retry: 1200\n\n"));
        yield Ok(web::Bytes::from_static(b"data: {\"heartbeat\":true}\n\n"));
        loop {
            let batch: Vec<(usize, SerializableEvent)> = {
                let events = lock_recover(&data.events, "events");
                web_ui_events::pull_events(&events, next_id, web_ui_events::MAX_EVENTS_PER_STREAM_BATCH)
            };
            if batch.is_empty() {
                yield Ok(web::Bytes::from_static(b"data: {\"heartbeat\":true}\n\n"));
            } else {
                for (id, evt) in batch {
                    next_id = next_id.max(id + 1);
                    let payload = serde_json::json!({ "id": id, "evt": evt });
                    let line = format!("id: {}\ndata: {}\n\n", id, payload);
                    yield Ok(web::Bytes::from(line));
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    };
    HttpResponse::Ok()
        .insert_header(("Content-Type", "text/event-stream"))
        .insert_header(("Cache-Control", "no-cache, no-transform"))
        .insert_header(("Connection", "keep-alive"))
        .insert_header(("X-Accel-Buffering", "no"))
        .streaming(s)
}

async fn index_page(req: HttpRequest, data: web::Data<AppState>) -> impl Responder {
    // If there's no valid session cookie, send the user to /login.
    let sid = auth::session::read_session_cookie(&req).unwrap_or_default();
    let ok = data.auth.resolve_session(&sid).is_some();
    if !ok {
        return HttpResponse::Found()
            .insert_header(("Location", "/login"))
            .finish();
    }
    HttpResponse::Ok()
        .insert_header(("Cache-Control", "no-store"))
        .content_type("text/html; charset=utf-8")
        .body(INDEX_HTML)
}

async fn diff_page(req: HttpRequest, data: web::Data<AppState>) -> impl Responder {
    let sid = auth::session::read_session_cookie(&req).unwrap_or_default();
    let ok = data.auth.resolve_session(&sid).is_some();
    if !ok {
        return HttpResponse::Found()
            .insert_header(("Location", "/login"))
            .finish();
    }
    HttpResponse::Ok()
        .insert_header(("Cache-Control", "no-store"))
        .content_type("text/html; charset=utf-8")
        .body(DIFF_HTML)
}

async fn app_css() -> impl Responder {
    HttpResponse::Ok()
        .insert_header(("Cache-Control", "no-store"))
        .content_type("text/css; charset=utf-8")
        .body(APP_CSS)
}

async fn app_js() -> impl Responder {
    HttpResponse::Ok()
        .insert_header(("Cache-Control", "no-store"))
        .content_type("application/javascript; charset=utf-8")
        .body(APP_JS)
}

async fn diff_js() -> impl Responder {
    HttpResponse::Ok()
        .insert_header(("Cache-Control", "no-store"))
        .content_type("application/javascript; charset=utf-8")
        .body(DIFF_JS)
}

async fn list_languages() -> impl Responder {
    match list_language_packs() {
        Ok(languages) => HttpResponse::Ok()
            .insert_header(("Cache-Control", "no-store"))
            .json(LanguageListResponse { languages }),
        Err(e) => HttpResponse::InternalServerError().body(e),
    }
}

async fn get_language_pack(path: web::Path<String>) -> impl Responder {
    let code = path.into_inner();
    if sanitize_language_code(&code).is_none() {
        return HttpResponse::BadRequest().body("invalid language code");
    }
    match read_language_pack(&code) {
        Ok(content) => HttpResponse::Ok()
            .insert_header(("Cache-Control", "no-store"))
            .content_type("application/json; charset=utf-8")
            .body(content),
        Err(e) if e == "language not found" => HttpResponse::NotFound().body(e),
        Err(e) => HttpResponse::InternalServerError().body(e),
    }
}

pub(crate) fn read_gate_threshold() -> u8 {
    FIXED_RELEASE_GATE_THRESHOLD
}

pub(crate) fn release_gate(
    readiness_score: u8,
    gate_threshold: u8,
    blockers: &[String],
    running: bool,
) -> (bool, String) {
    web_ui_analytics::release_gate(readiness_score, gate_threshold, blockers, running)
}

#[derive(serde::Deserialize)]
struct DiffDataQuery {
    bucket: String,
}

#[derive(serde::Serialize)]
struct DiffDataResponse {
    ok: bool,
    bucket: String,
    project_label: String,
    run_state: String,
    run_text: String,
    diff_text: String,
}

async fn diff_data(
    req: HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<DiffDataQuery>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let bucket = query.bucket.trim().to_string();
    if bucket.is_empty() || bucket.len() > 512 {
        return HttpResponse::BadRequest().body("invalid bucket");
    }
    let (project_label, project_workspace, run_state, run_text) = {
        let _guard = lock_recover(&data.projects_lock, "projects_lock");
        let cache = read_ui_cache_for_root(&ctx.managed_root_dir);
        let mut project_label = String::new();
        let mut project_workspace: Option<String> = None;
        if let Some(id) = bucket.strip_prefix("project:") {
            if let Some(p) = cache.projects.iter().find(|p| p.id == id) {
                project_workspace = Some(p.workspace.clone());
                project_label = if !p.name.trim().is_empty() {
                    p.name.trim().to_string()
                } else if !p.workspace.trim().is_empty() {
                    p.workspace.trim().to_string()
                } else {
                    id.to_string()
                };
            } else {
                project_label = id.to_string();
            }
        }

        let mut run_state = "idle".to_string();
        let mut run_text = String::new();
        if let Some(v) = cache.project_ui_state.get(&bucket) {
            if let Some(obj) = v.as_object() {
                if let Some(s) = obj.get("run_state").and_then(|x| x.as_str()) {
                    run_state = s.to_string();
                }
                if let Some(s) = obj.get("run_text").and_then(|x| x.as_str()) {
                    run_text = s.to_string();
                }
            }
        }
        (project_label, project_workspace, run_state, run_text)
    };

    let diff_text = project_workspace
        .as_deref()
        .and_then(|ws| require_managed_workspace_for_root(ws, &ctx.managed_root_dir).ok())
        .and_then(|path| crate::git_utils::diff_last_commit(&path).ok())
        .unwrap_or_default();

    HttpResponse::Ok().json(DiffDataResponse {
        ok: true,
        bucket,
        project_label,
        run_state,
        run_text,
        diff_text,
    })
}

pub async fn run_web_server(
    tx_req: Sender<AgentRequest>,
    rx_evt: Receiver<AgentEvent>,
    stop_now: Arc<AtomicBool>,
) -> std::io::Result<()> {
    fs::create_dir_all(managed_root_path())?;
    load_language_packs_checked().map_err(|e| {
        eprintln!("[KACF] ERROR: {}", e);
        std::io::Error::new(std::io::ErrorKind::InvalidData, e)
    })?;
    let auth_paths = auth::AuthSystemPaths::new(&managed_root_path());
    let auth_store = auth::AuthStore::new(auth_paths);
    auth_store.ensure_dirs()?;
    let port = std::env::var("AUTOCODING_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(8080);
    let state = AppState {
        tx_req,
        events: Arc::new(Mutex::new(Default::default())),
        event_bytes: Arc::new(Mutex::new(0)),
        next_event_id: Arc::new(Mutex::new(0)),
        runtime: Arc::new(Mutex::new(RuntimeStatus::default())),
        projects_lock: Arc::new(Mutex::new(())),
        debug_client_logs: web_ui_debug::DebugLogStore::new(),
        stop_now,
        auth: auth_store,
    };
    web_ui_events::spawn_event_collector(
        rx_evt,
        state.runtime.clone(),
        state.events.clone(),
        state.event_bytes.clone(),
        state.next_event_id.clone(),
    );

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(state.clone()))
            .route("/", web::get().to(index_page))
            .route("/diff", web::get().to(diff_page))
            .route("/login", web::get().to(auth::login_page))
            .route("/account", web::get().to(auth::account_page))
            .route("/admin", web::get().to(auth::admin_page))
            .route("/assets/app.css", web::get().to(app_css))
            .route("/assets/app.js", web::get().to(app_js))
            .route("/assets/diff.js", web::get().to(diff_js))
            .route("/assets/auth.js", web::get().to(auth::auth_js))
            .route("/assets/account.js", web::get().to(auth::account_js))
            .route("/assets/admin.js", web::get().to(auth::admin_js))
            .route("/assets/languages/list", web::get().to(list_languages))
            .route(
                "/assets/languages/{code}.json",
                web::get().to(get_language_pack),
            )
            .route(
                "/auth/bootstrap_status",
                web::get().to(auth::bootstrap_status),
            )
            .route(
                "/auth/bootstrap_admin",
                web::post().to(auth::bootstrap_admin),
            )
            .route("/auth/register", web::post().to(auth::register))
            .route("/auth/login", web::post().to(auth::login))
            .route(
                "/auth/verify_email_code",
                web::post().to(auth::verify_email_code),
            )
            .route("/auth/guest", web::post().to(auth::guest_start))
            .route("/auth/logout", web::post().to(auth::logout))
            .route("/auth/me", web::get().to(auth::auth_me))
            .route(
                "/admin/api/settings",
                web::get().to(auth::admin_get_settings),
            )
            .route(
                "/admin/api/settings",
                web::post().to(auth::admin_put_settings),
            )
            .route("/admin/api/users", web::get().to(auth::admin_list_users))
            .route("/admin/api/users", web::post().to(auth::admin_create_user))
            .route(
                "/admin/api/users/{username}",
                web::delete().to(auth::admin_delete_user),
            )
            .route(
                "/admin/api/users/{username}/ban",
                web::post().to(auth::admin_set_banned),
            )
            .route(
                "/admin/api/users/{username}/notice",
                web::post().to(auth::admin_set_notice),
            )
            .route(
                "/admin/api/users/{username}/password",
                web::post().to(auth::admin_set_password),
            )
            .route("/admin/api/audit", web::get().to(auth::admin_audit_tail))
            .route(
                "/account/api/profile",
                web::post().to(auth::account_update_profile),
            )
            .route(
                "/account/api/password",
                web::post().to(auth::account_change_password),
            )
            .route("/account/api/self", web::get().to(auth::account_self))
            .route(
                "/account/api/request_current_email_code",
                web::post().to(auth::account_request_current_email_code),
            )
            .route(
                "/account/api/change_username",
                web::post().to(auth::account_change_username),
            )
            .route(
                "/account/api/request_new_email_code",
                web::post().to(auth::account_request_new_email_code),
            )
            .route(
                "/account/api/confirm_email_change",
                web::post().to(auth::account_confirm_email_change),
            )
            .route(
                "/account/api/change_login_option",
                web::post().to(auth::account_change_login_option),
            )
            .route("/start", web::post().to(web_ui_session::start_session))
            .route("/stop", web::post().to(web_ui_session::stop_session))
            .route("/resume", web::post().to(web_ui_session::resume_session))
            .route(
                "/project_config",
                web::get().to(web_ui_session::get_project_config),
            )
            .route("/projects", web::get().to(list_projects))
            .route("/projects", web::post().to(upsert_project))
            .route("/projects/{id}", web::delete().to(delete_project))
            .route(
                "/projects/suggest_slug",
                web::post().to(suggest_project_slug),
            )
            .route("/ui_cache", web::get().to(get_ui_cache))
            .route("/ui_cache", web::put().to(put_ui_cache))
            .route("/ui_state", web::get().to(web_ui_session::get_ui_state))
            .route("/diff_data", web::get().to(diff_data))
            .route("/vm/status", web::get().to(web_ui_vm::get_vm_status))
            .route("/vm/logs", web::get().to(web_ui_vm::get_vm_logs))
            .route("/vm/snapshot/list", web::get().to(web_ui_vm::list_vm_snapshots))
            .route("/vm/snapshot/create", web::post().to(web_ui_vm::create_vm_snapshot))
            .route("/vm/snapshot/apply", web::post().to(web_ui_vm::apply_vm_snapshot))
            .route("/vm/snapshot/delete", web::post().to(web_ui_vm::delete_vm_snapshot))
            .route("/vm/clone", web::post().to(web_ui_vm::clone_vm_from_snapshot))
            .route("/vm/exec", web::post().to(web_ui_vm::exec_in_vm))
            .route("/vm/exec/cancel", web::post().to(web_ui_vm::cancel_vm_exec))
            .route("/vm/bootstrap", web::post().to(web_ui_vm::bootstrap_vm))
            .route("/vm/exec/queue", web::get().to(web_ui_vm::list_vm_exec_queue))
            .route("/vm/exec/queue/stats", web::get().to(web_ui_vm::vm_exec_queue_stats))
            .route("/vm/exec/profiles", web::get().to(web_ui_vm::list_vm_exec_profiles))
            .route(
                "/vm/exec/profile/detail",
                web::get().to(web_ui_vm::get_vm_exec_profile_detail),
            )
            .route(
                "/vm/exec/profile/preview",
                web::get().to(web_ui_vm::preview_vm_exec_profile),
            )
            .route(
                "/vm/exec/profiles/save",
                web::post().to(web_ui_vm::save_vm_exec_custom_profile),
            )
            .route(
                "/vm/exec/profiles/delete",
                web::post().to(web_ui_vm::delete_vm_exec_custom_profile),
            )
            .route("/vm/exec/enqueue", web::post().to(web_ui_vm::enqueue_vm_exec))
            .route(
                "/vm/exec/enqueue_batch",
                web::post().to(web_ui_vm::enqueue_vm_exec_batch),
            )
            .route(
                "/vm/exec/enqueue_profile",
                web::post().to(web_ui_vm::enqueue_vm_exec_profile),
            )
            .route(
                "/vm/self_debug/plan",
                web::post().to(web_ui_vm::preview_vm_self_debug_plan),
            )
            .route(
                "/vm/self_debug/start",
                web::post().to(web_ui_vm::start_vm_self_debug_plan),
            )
            .route(
                "/vm/self_debug/runs",
                web::get().to(web_ui_vm::list_vm_self_debug_runs),
            )
            .route(
                "/vm/self_debug/history",
                web::get().to(web_ui_vm::list_vm_self_debug_history),
            )
            .route(
                "/vm/self_debug/history/detail",
                web::get().to(web_ui_vm::get_vm_self_debug_history_detail),
            )
            .route(
                "/vm/self_debug/history/archive_completed",
                web::post().to(web_ui_vm::archive_vm_self_debug_history),
            )
            .route(
                "/vm/self_debug/history/clear",
                web::post().to(web_ui_vm::clear_vm_self_debug_history),
            )
            .route(
                "/vm/self_debug/run_detail",
                web::get().to(web_ui_vm::get_vm_self_debug_run_detail),
            )
            .route(
                "/vm/self_debug/context",
                web::get().to(web_ui_vm::get_vm_self_debug_context),
            )
            .route(
                "/vm/self_debug/strategy_stats",
                web::get().to(web_ui_vm::get_vm_self_debug_strategy_stats),
            )
            .route(
                "/vm/self_debug/strategy_rules",
                web::get().to(web_ui_vm::get_vm_self_debug_strategy_rules),
            )
            .route(
                "/vm/self_debug/strategy_rules",
                web::post().to(web_ui_vm::save_vm_self_debug_strategy_rules),
            )
            .route(
                "/vm/self_debug/stop",
                web::post().to(web_ui_vm::stop_vm_self_debug_run),
            )
            .route(
                "/vm/self_debug/pause",
                web::post().to(web_ui_vm::pause_vm_self_debug_run),
            )
            .route(
                "/vm/self_debug/resume",
                web::post().to(web_ui_vm::resume_vm_self_debug_run),
            )
            .route("/vm/exec/queue/cancel", web::post().to(web_ui_vm::cancel_vm_exec_task))
            .route("/vm/exec/queue/run_next", web::post().to(web_ui_vm::run_next_vm_exec))
            .route("/vm/exec/dispatch", web::post().to(web_ui_vm::dispatch_vm_exec))
            .route("/vm/ready", web::get().to(web_ui_vm::check_vm_ready))
            .route("/vm/provision", web::post().to(web_ui_vm::provision_vm))
            .route("/vm/start", web::post().to(web_ui_vm::start_vm))
            .route("/vm/stop", web::post().to(web_ui_vm::stop_vm))
            .route("/vm/delete", web::post().to(web_ui_vm::delete_vm))
            .route("/health", web::get().to(health))
            .route("/metrics", web::get().to(metrics))
            .route("/clarify", web::post().to(web_ui_session::answer_clarify))
            .route("/revert", web::post().to(web_ui_session::revert_last))
            .route("/push", web::post().to(web_ui_session::push_remote))
            .route("/events", web::get().to(get_events))
            .route("/events/stream", web::get().to(stream_events))
            .service(
                web::scope("/debug")
                    .app_data(web::Data::new(state.debug_client_logs.entries()))
                    .app_data(web::Data::new(state.debug_client_logs.bytes()))
                    .route(
                        "/client_logs",
                        web::post().to(web_ui_debug::post_client_log),
                    )
                    .route("/client_logs", web::get().to(web_ui_debug::get_client_logs)),
            )
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
}

#[cfg(test)]
mod tests {
    use super::release_gate;

    #[test]
    fn gate_fails_when_blockers_exist() {
        let blockers = vec!["存在失败".to_string()];
        let (passed, reason) = release_gate(90, 75, &blockers, false);
        assert!(!passed);
        assert!(reason.contains("存在失败"));
    }

    #[test]
    fn gate_fails_when_score_too_low() {
        let (passed, reason) = release_gate(60, 75, &[], false);
        assert!(!passed);
        assert!(reason.contains("低于阈值"));
    }

    #[test]
    fn gate_passes_for_good_score_without_blockers() {
        let (passed, reason) = release_gate(85, 75, &[], false);
        assert!(passed);
        assert!(reason.contains("通过"));
    }

    #[test]
    fn readiness_score_penalizes_failures() {
        let score = crate::web_ui_analytics::readiness_score(
            &super::RuntimeStatus::default(),
            0,
            2,
            Some(50.0),
            Some(1000),
            Some(1000),
        );
        assert!(score < 75);
    }
}
