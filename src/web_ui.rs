//! Minimal web UI mode for the KACF (Kevin AutoCoding Framework) agent.
//!
//! This module exposes HTTP endpoints to control the agent loop and to
//! retrieve events (logs, diffs, clarifications, completion). It is
//! intended to allow running the system on machines without a native
//! desktop environment (e.g. Termux) and interacting through a
//! browser. The server listens on 0.0.0.0:8080 by default.

use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use async_stream::stream;
use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::deepseek_api;
use crate::protocol::{AgentEvent, AgentRequest, ClarifyAnswer};
use crate::web_ui_analytics::{self, CategoryCount, TimePoint};
use crate::web_ui_cache_logic;
use crate::web_ui_events::{self, SerializableEvent};
use crate::web_ui_languages;
use crate::web_ui_projects;
use crate::web_ui_runtime_env;
use crate::web_ui_runtime_metrics;
use crate::web_ui_slug;
use crate::web_ui_store;

/// Index HTML page embedded at compile time.
const INDEX_HTML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/static/index.html"));
const APP_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/static/app.css"));
const APP_JS: &str = concat!(
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/static/js/app_state.js"
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
        "/static/js/app_runtime_actions.js"
    ))
);

const SESSION_STATE_FILENAME: &str = ".autocoding_state.json";
const PROJECT_CONFIG_FILENAME: &str = ".autocoding_project.json";
const UI_CACHE_FILENAME: &str = ".autocoding_webui_cache.json";
const MANAGED_ROOT_DIR: &str = "autocoding_data";
const MANAGED_WORKSPACES_DIR: &str = "workspaces";

#[derive(Clone)]
pub struct AppState {
    tx_req: Sender<AgentRequest>,
    events: Arc<Mutex<Vec<(usize, SerializableEvent)>>>,
    event_bytes: Arc<Mutex<usize>>,
    next_event_id: Arc<Mutex<usize>>,
    runtime: Arc<Mutex<RuntimeStatus>>,
    projects_lock: Arc<Mutex<()>>,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
struct StartPayload {
    api_key: String,
    base_url: String,
    model: String,
    #[serde(default)]
    language: String,
    #[serde(default = "default_auto_revert_profile")]
    auto_revert_profile: String,
    #[serde(default)]
    unattended_mode: bool,
    #[serde(default)]
    precheck_cmd: String,
    #[serde(default)]
    release_gate_threshold: String,
    workspace: String,
    goal: String,
    eval_cmd: String,
    success_regex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DraftPayload {
    api_key: String,
    base_url: String,
    model: String,
    #[serde(default)]
    language: String,
    #[serde(default = "default_auto_revert_profile")]
    auto_revert_profile: String,
    #[serde(default)]
    unattended_mode: bool,
    #[serde(default)]
    precheck_cmd: String,
    #[serde(default)]
    release_gate_threshold: String,
    workspace: String,
    goal: String,
    eval_cmd: String,
    success_regex: String,
    remote: String,
    remote_url: String,
    branch: String,
}

#[derive(Debug, Deserialize)]
struct ProjectConfigQuery {
    workspace: String,
}

#[derive(Debug, Deserialize)]
struct ResumePayload {
    #[serde(default)]
    project_id: String,
    #[serde(default)]
    unattended_mode: bool,
}

impl DraftPayload {
    fn into_start(self) -> StartPayload {
        StartPayload {
            api_key: self.api_key,
            base_url: self.base_url,
            model: self.model,
            language: self.language,
            auto_revert_profile: self.auto_revert_profile,
            unattended_mode: self.unattended_mode,
            precheck_cmd: self.precheck_cmd,
            release_gate_threshold: self.release_gate_threshold,
            workspace: self.workspace,
            goal: self.goal,
            eval_cmd: self.eval_cmd,
            success_regex: self.success_regex,
        }
    }
}

fn default_auto_revert_profile() -> String {
    "balanced".to_string()
}

#[derive(Debug, Deserialize)]
struct ClarifyPayload {
    answers: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct PushPayload {
    remote: String,
    url: String,
    branch: String,
}

#[derive(Debug, Deserialize)]
struct SlugSuggestPayload {
    #[serde(default)]
    project_name: String,
    #[serde(default)]
    goal: String,
    #[serde(default)]
    api_key: String,
    #[serde(default = "default_base_url")]
    base_url: String,
    #[serde(default = "default_model_name")]
    model: String,
}

fn default_base_url() -> String {
    "https://api.deepseek.com".to_string()
}

fn default_model_name() -> String {
    "deepseek-reasoner".to_string()
}

#[derive(Debug, Serialize)]
struct SlugSuggestResponse {
    slug: String,
    source: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub(crate) struct RuntimeStatus {
    pub(crate) running: bool,
    pub(crate) last_start_unix: u64,
    pub(crate) last_workspace: String,
    pub(crate) last_goal: String,
    pub(crate) total_events: u64,
    pub(crate) total_logs: u64,
    pub(crate) total_done_ok: u64,
    pub(crate) total_done_fail: u64,
    pub(crate) last_error: String,
    #[serde(skip_serializing)]
    pub(crate) api_ms_samples: Vec<u32>,
    #[serde(skip_serializing)]
    pub(crate) eval_ms_samples: Vec<u32>,
    #[serde(skip_serializing)]
    pub(crate) done_history: Vec<(u64, bool)>,
    #[serde(skip_serializing)]
    pub(crate) digest_history: Vec<(u64, String)>,
    #[serde(skip_serializing)]
    pub(crate) root_cause_history: Vec<(u64, String)>,
}

#[derive(Debug, Clone, Serialize)]
struct ResumeInfo {
    resumable: bool,
    updated_at_unix: Option<u64>,
    iteration: Option<u32>,
    message_count: Option<usize>,
    last_status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResumeMeta {
    #[serde(default)]
    goal: String,
    #[serde(default)]
    updated_at_unix: Option<u64>,
    #[serde(default)]
    iteration: Option<u32>,
    #[serde(default)]
    message_count: Option<usize>,
    #[serde(default)]
    last_status: Option<String>,
}

#[derive(Debug, Serialize)]
struct UiStateResponse {
    runtime: RuntimeStatus,
    resume: Option<ResumeInfo>,
}

#[derive(Debug, Serialize)]
struct LanguageListResponse {
    languages: Vec<String>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    service: &'static str,
    version: &'static str,
    unix_time: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct MetricsResponse {
    pub(crate) unix_time: u64,
    pub(crate) event_buffer_len: usize,
    pub(crate) next_event_id: usize,
    pub(crate) running: bool,
    pub(crate) total_events: u64,
    pub(crate) total_logs: u64,
    pub(crate) total_done_ok: u64,
    pub(crate) total_done_fail: u64,
    pub(crate) last_error: String,
    pub(crate) api_p50_ms: Option<u32>,
    pub(crate) api_p95_ms: Option<u32>,
    pub(crate) eval_p50_ms: Option<u32>,
    pub(crate) eval_p95_ms: Option<u32>,
    pub(crate) done_5m_ok: u64,
    pub(crate) done_5m_fail: u64,
    pub(crate) done_5m_success_rate: Option<f64>,
    pub(crate) readiness: String,
    pub(crate) readiness_score: u8,
    pub(crate) blockers: Vec<String>,
    pub(crate) actions: Vec<String>,
    pub(crate) gate_threshold: u8,
    pub(crate) gate_passed: bool,
    pub(crate) gate_reason: String,
    pub(crate) digest_5m: Vec<CategoryCount>,
    pub(crate) root_causes_5m: Vec<CategoryCount>,
    pub(crate) success_rate_series_5m: Vec<TimePoint>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ProjectConfig {
    auto_revert_profile: String,
    #[serde(default)]
    unattended_mode: bool,
    #[serde(default)]
    precheck_cmd: String,
    #[serde(default)]
    release_gate_threshold: String,
    updated_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WebProject {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) workspace: String,
    pub(crate) goal: String,
    pub(crate) updated_at: u64,
    snapshot: DraftPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct UiCachePayload {
    #[serde(default)]
    pub(crate) projects: Vec<WebProject>,
    #[serde(default)]
    pub(crate) shared_config: Option<SharedConfig>,
    #[serde(default)]
    pub(crate) global_options: Option<GlobalOptions>,
    #[serde(default)]
    pub(crate) project_logs: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) project_ui_state: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct SharedConfig {
    #[serde(default)]
    pub(crate) api_key: String,
    #[serde(default)]
    pub(crate) base_url: String,
    #[serde(default)]
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct GlobalOptions {
    #[serde(default)]
    pub(crate) auto_resume_attempts: String,
    #[serde(default)]
    pub(crate) stop_after_minutes: String,
    #[serde(default)]
    pub(crate) history_max_messages: String,
    #[serde(default)]
    pub(crate) history_max_chars: String,
    #[serde(default)]
    pub(crate) log_max_chars: String,
    #[serde(default)]
    pub(crate) diff_max_chars: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct UiCachePatch {
    #[serde(default)]
    pub(crate) projects: Option<Vec<WebProject>>,
    #[serde(default)]
    pub(crate) shared_config: Option<SharedConfig>,
    #[serde(default)]
    pub(crate) global_options: Option<GlobalOptions>,
    #[serde(default)]
    pub(crate) project_logs: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) project_ui_state: Option<std::collections::BTreeMap<String, serde_json::Value>>,
}

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

fn managed_workspaces_path() -> PathBuf {
    managed_root_path().join(MANAGED_WORKSPACES_DIR)
}

fn merge_shared_config_into_draft(draft: &mut DraftPayload, cfg: &SharedConfig) {
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

fn normalize_resume_draft_defaults(draft: &mut DraftPayload) {
    if draft.base_url.trim().is_empty() {
        draft.base_url = default_base_url();
    }
    if draft.model.trim().is_empty() {
        draft.model = default_model_name();
    }
}

fn require_managed_workspace(workspace: &str) -> Result<PathBuf, String> {
    web_ui_store::require_managed_workspace(workspace, MANAGED_ROOT_DIR, MANAGED_WORKSPACES_DIR)
}

fn save_project_config(
    workspace: &str,
    profile: &str,
    unattended_mode: bool,
    precheck_cmd: &str,
    release_gate_threshold: &str,
) -> std::io::Result<()> {
    let cfg = ProjectConfig {
        auto_revert_profile: profile.to_string(),
        unattended_mode,
        precheck_cmd: precheck_cmd.to_string(),
        release_gate_threshold: release_gate_threshold.to_string(),
        updated_at_unix: now_unix(),
    };
    web_ui_store::save_project_config(
        workspace,
        MANAGED_ROOT_DIR,
        MANAGED_WORKSPACES_DIR,
        PROJECT_CONFIG_FILENAME,
        &cfg,
    )
}

fn read_project_config(workspace: &str) -> Option<ProjectConfig> {
    web_ui_store::read_project_config(
        workspace,
        MANAGED_ROOT_DIR,
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

fn read_ui_cache() -> UiCachePayload {
    web_ui_store::read_ui_cache(MANAGED_ROOT_DIR, UI_CACHE_FILENAME)
}

fn write_ui_cache(payload: &UiCachePayload) -> std::io::Result<()> {
    web_ui_store::write_ui_cache(MANAGED_ROOT_DIR, UI_CACHE_FILENAME, payload)
}

fn read_global_history_limits(opts: Option<&GlobalOptions>) -> (String, String) {
    let Some(opts) = opts else {
        return (String::new(), String::new());
    };
    (
        opts.history_max_messages.trim().to_string(),
        opts.history_max_chars.trim().to_string(),
    )
}

fn read_resume_info(workspace: &str, goal: &str) -> Option<ResumeInfo> {
    let path = require_managed_workspace(workspace)
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

fn start_from_payload(
    data: &web::Data<AppState>,
    payload: StartPayload,
    global_history_max_messages: &str,
    global_history_max_chars: &str,
    resume_from_checkpoint: bool,
) -> Result<(), String> {
    if payload.api_key.trim().is_empty() {
        return Err("api_key is empty".to_string());
    }
    {
        let runtime = data.runtime.lock().unwrap();
        if runtime.running {
            return Err("session already running".to_string());
        }
    }
    let auto_revert_profile = payload.auto_revert_profile.clone();
    let workspace_full = require_managed_workspace(&payload.workspace)?;
    web_ui_runtime_env::apply_runtime_config_envs(
        &payload.precheck_cmd,
        global_history_max_messages,
        global_history_max_chars,
        &payload.release_gate_threshold,
    );
    let req = AgentRequest::Start {
        api_key: payload.api_key,
        base_url: payload.base_url,
        model: payload.model,
        language: payload.language,
        auto_revert_profile: auto_revert_profile.clone(),
        unattended_mode: payload.unattended_mode,
        resume_from_checkpoint,
        workspace: workspace_full.clone(),
        goal: payload.goal.clone(),
        eval_cmd: payload.eval_cmd,
        success_regex: payload.success_regex,
    };
    if let Err(e) = save_project_config(
        &payload.workspace,
        &auto_revert_profile,
        payload.unattended_mode,
        &payload.precheck_cmd,
        &payload.release_gate_threshold,
    ) {
        eprintln!("save project config failed: {}", e);
    }
    data.tx_req
        .send(req)
        .map_err(|e| format!("send start failed: {}", e))?;
    let mut runtime = data.runtime.lock().unwrap();
    runtime.running = true;
    runtime.last_start_unix = now_unix();
    runtime.last_workspace = payload.workspace;
    runtime.last_goal = payload.goal;
    runtime.last_error.clear();
    Ok(())
}

async fn start_session(data: web::Data<AppState>, body: web::Json<StartPayload>) -> impl Responder {
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

async fn get_project_config(query: web::Query<ProjectConfigQuery>) -> impl Responder {
    match read_project_config(&query.workspace) {
        Some(cfg) => HttpResponse::Ok().json(cfg),
        None => HttpResponse::NotFound().body("project config not found"),
    }
}

async fn list_projects(data: web::Data<AppState>) -> impl Responder {
    let _guard = data.projects_lock.lock().unwrap();
    let mut items = read_ui_cache().projects;
    web_ui_projects::sort_projects_by_updated_desc(&mut items);
    HttpResponse::Ok().json(items)
}

async fn upsert_project(data: web::Data<AppState>, body: web::Json<WebProject>) -> impl Responder {
    let _guard = data.projects_lock.lock().unwrap();
    let item = match web_ui_projects::normalize_project_for_upsert(body.into_inner(), now_unix()) {
        Ok(v) => v,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };
    let mut cache = read_ui_cache();
    web_ui_projects::upsert_project_in_cache(&mut cache, item.clone());
    if let Err(e) = write_ui_cache(&cache) {
        return HttpResponse::InternalServerError().body(format!("write projects failed: {}", e));
    }
    HttpResponse::Ok().json(item)
}

async fn delete_project(data: web::Data<AppState>, path: web::Path<String>) -> impl Responder {
    let _guard = data.projects_lock.lock().unwrap();
    let id = path.into_inner();
    let mut cache = read_ui_cache();
    if let Err(e) = web_ui_projects::delete_project_from_cache(&mut cache, &id) {
        return if e == "project id is empty" {
            HttpResponse::BadRequest().body(e)
        } else {
            HttpResponse::NotFound().body(e)
        };
    }
    if let Err(e) = write_ui_cache(&cache) {
        return HttpResponse::InternalServerError().body(format!("write projects failed: {}", e));
    }
    HttpResponse::Ok().body("deleted")
}

async fn get_ui_cache(data: web::Data<AppState>) -> impl Responder {
    let _guard = data.projects_lock.lock().unwrap();
    let payload = read_ui_cache();
    HttpResponse::Ok().json(payload)
}

async fn put_ui_cache(data: web::Data<AppState>, body: web::Json<UiCachePatch>) -> impl Responder {
    let _guard = data.projects_lock.lock().unwrap();
    let patch = body.into_inner();
    let mut payload = read_ui_cache();
    web_ui_cache_logic::apply_ui_cache_patch(&mut payload, patch);
    match write_ui_cache(&payload) {
        Ok(_) => HttpResponse::Ok().body("saved"),
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

async fn resume_session(
    data: web::Data<AppState>,
    body: web::Json<ResumePayload>,
) -> impl Responder {
    let payload = body.into_inner();
    if payload.project_id.trim().is_empty() {
        return HttpResponse::BadRequest().body("project_id is empty");
    }
    let (mut draft, history_max_messages, history_max_chars): (DraftPayload, String, String) = {
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

async fn get_ui_state(data: web::Data<AppState>) -> impl Responder {
    let runtime = data.runtime.lock().unwrap().clone();
    let resume = if runtime.last_workspace.trim().is_empty() {
        None
    } else {
        read_resume_info(&runtime.last_workspace, &runtime.last_goal)
    };
    HttpResponse::Ok().json(UiStateResponse { runtime, resume })
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
    let runtime = data.runtime.lock().unwrap().clone();
    let events_len = data.events.lock().unwrap().len();
    let next_id = *data.next_event_id.lock().unwrap();
    let body =
        web_ui_runtime_metrics::build_metrics_response(&runtime, events_len, next_id, now_unix());
    HttpResponse::Ok().json(body)
}

async fn answer_clarify(
    data: web::Data<AppState>,
    body: web::Json<ClarifyPayload>,
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

async fn push_remote(data: web::Data<AppState>, body: web::Json<PushPayload>) -> impl Responder {
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

async fn revert_last(data: web::Data<AppState>) -> impl Responder {
    if let Err(e) = data.tx_req.send(AgentRequest::RevertLast) {
        return HttpResponse::InternalServerError().body(format!("send revert failed: {}", e));
    }
    HttpResponse::Ok().body("revert sent")
}

async fn stop_session(data: web::Data<AppState>) -> impl Responder {
    let mut runtime = data.runtime.lock().unwrap();
    runtime.running = false;
    match data.tx_req.send(AgentRequest::Stop) {
        Ok(_) => HttpResponse::Ok().body("stop sent"),
        Err(e) => {
            runtime.last_error = format!("stop channel unavailable: {}", e);
            HttpResponse::Ok().body("stop acknowledged (channel unavailable)")
        }
    }
}

async fn get_events(
    data: web::Data<AppState>,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> impl Responder {
    let from_id: usize = query.get("from").and_then(|v| v.parse().ok()).unwrap_or(0);
    let events = data.events.lock().unwrap();
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
                let events = data.events.lock().unwrap();
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

async fn index_page() -> impl Responder {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(INDEX_HTML)
}

async fn app_css() -> impl Responder {
    HttpResponse::Ok()
        .content_type("text/css; charset=utf-8")
        .body(APP_CSS)
}

async fn app_js() -> impl Responder {
    HttpResponse::Ok()
        .content_type("application/javascript; charset=utf-8")
        .body(APP_JS)
}

async fn list_languages() -> impl Responder {
    match list_language_packs() {
        Ok(languages) => HttpResponse::Ok().json(LanguageListResponse { languages }),
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
            .content_type("application/json; charset=utf-8")
            .body(content),
        Err(e) if e == "language not found" => HttpResponse::NotFound().body(e),
        Err(e) => HttpResponse::InternalServerError().body(e),
    }
}

pub(crate) fn read_gate_threshold() -> u8 {
    std::env::var("AUTOCODING_RELEASE_GATE_THRESHOLD")
        .ok()
        .and_then(|v| v.parse::<u8>().ok())
        .filter(|v| *v <= 100)
        .unwrap_or(75)
}

pub(crate) fn release_gate(
    readiness_score: u8,
    gate_threshold: u8,
    blockers: &[String],
    running: bool,
) -> (bool, String) {
    web_ui_analytics::release_gate(readiness_score, gate_threshold, blockers, running)
}

pub async fn run_web_server(
    tx_req: Sender<AgentRequest>,
    rx_evt: Receiver<AgentEvent>,
) -> std::io::Result<()> {
    fs::create_dir_all(managed_root_path())?;
    fs::create_dir_all(managed_workspaces_path())?;
    load_language_packs_checked().map_err(|e| {
        eprintln!("[KACF] ERROR: {}", e);
        std::io::Error::new(std::io::ErrorKind::InvalidData, e)
    })?;
    let port = std::env::var("AUTOCODING_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(8080);
    let state = AppState {
        tx_req,
        events: Arc::new(Mutex::new(Vec::new())),
        event_bytes: Arc::new(Mutex::new(0)),
        next_event_id: Arc::new(Mutex::new(0)),
        runtime: Arc::new(Mutex::new(RuntimeStatus::default())),
        projects_lock: Arc::new(Mutex::new(())),
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
            .route("/assets/app.css", web::get().to(app_css))
            .route("/assets/app.js", web::get().to(app_js))
            .route("/assets/languages/list", web::get().to(list_languages))
            .route(
                "/assets/languages/{code}.json",
                web::get().to(get_language_pack),
            )
            .route("/start", web::post().to(start_session))
            .route("/stop", web::post().to(stop_session))
            .route("/resume", web::post().to(resume_session))
            .route("/project_config", web::get().to(get_project_config))
            .route("/projects", web::get().to(list_projects))
            .route("/projects", web::post().to(upsert_project))
            .route("/projects/{id}", web::delete().to(delete_project))
            .route(
                "/projects/suggest_slug",
                web::post().to(suggest_project_slug),
            )
            .route("/ui_cache", web::get().to(get_ui_cache))
            .route("/ui_cache", web::put().to(put_ui_cache))
            .route("/ui_state", web::get().to(get_ui_state))
            .route("/health", web::get().to(health))
            .route("/metrics", web::get().to(metrics))
            .route("/clarify", web::post().to(answer_clarify))
            .route("/revert", web::post().to(revert_last))
            .route("/push", web::post().to(push_remote))
            .route("/events", web::get().to(get_events))
            .route("/events/stream", web::get().to(stream_events))
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
