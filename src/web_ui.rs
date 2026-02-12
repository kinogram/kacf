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
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::deepseek_api;
use crate::protocol::{AgentEvent, AgentRequest, ClarifyAnswer, ClarifyQuestion};

/// Index HTML page embedded at compile time.
const INDEX_HTML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/static/index.html"));
const COPY_ZH_CN_JSON: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/static/copy.zh-CN.json"));

const DRAFT_FILENAME: &str = ".autocoding_webui_draft.json";
const SESSION_STATE_FILENAME: &str = ".autocoding_state.json";
const PROJECT_CONFIG_FILENAME: &str = ".autocoding_project.json";
const UI_CACHE_FILENAME: &str = ".autocoding_webui_cache.json";
const MANAGED_ROOT_DIR: &str = "autocoding_data";
const MANAGED_WORKSPACES_DIR: &str = "workspaces";
const MAX_EVENT_BUFFER: usize = 5000;

#[derive(Clone)]
pub struct AppState {
    tx_req: Sender<AgentRequest>,
    rx_evt: Receiver<AgentEvent>,
    events: Arc<Mutex<Vec<(usize, SerializableEvent)>>>,
    next_event_id: Arc<Mutex<usize>>,
    runtime: Arc<Mutex<RuntimeStatus>>,
    projects_lock: Arc<Mutex<()>>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SerializableEvent {
    Log { line: String },
    NeedClarify { questions: Vec<ClarifyQuestion> },
    Diff { diff: String },
    Done { success: bool, message: String },
}

impl From<AgentEvent> for SerializableEvent {
    fn from(evt: AgentEvent) -> Self {
        match evt {
            AgentEvent::Log(line) => SerializableEvent::Log { line },
            AgentEvent::NeedClarify { questions } => SerializableEvent::NeedClarify { questions },
            AgentEvent::Diff { diff } => SerializableEvent::Diff { diff },
            AgentEvent::Done { success, message } => SerializableEvent::Done { success, message },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StartPayload {
    api_key: String,
    base_url: String,
    model: String,
    #[serde(default = "default_auto_revert_profile")]
    auto_revert_profile: String,
    #[serde(default)]
    precheck_cmd: String,
    #[serde(default)]
    history_max_messages: String,
    #[serde(default)]
    history_max_chars: String,
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
    #[serde(default = "default_auto_revert_profile")]
    auto_revert_profile: String,
    #[serde(default)]
    precheck_cmd: String,
    #[serde(default)]
    history_max_messages: String,
    #[serde(default)]
    history_max_chars: String,
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
struct DraftQuery {
    #[serde(default)]
    workspace: String,
}

#[derive(Debug, Deserialize)]
struct ResumePayload {
    #[serde(default)]
    workspace: String,
    #[serde(default)]
    project_id: String,
}

impl DraftPayload {
    fn into_start(self) -> StartPayload {
        StartPayload {
            api_key: self.api_key,
            base_url: self.base_url,
            model: self.model,
            auto_revert_profile: self.auto_revert_profile,
            precheck_cmd: self.precheck_cmd,
            history_max_messages: self.history_max_messages,
            history_max_chars: self.history_max_chars,
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
struct RuntimeStatus {
    running: bool,
    last_start_unix: u64,
    last_workspace: String,
    last_goal: String,
    total_events: u64,
    total_logs: u64,
    total_done_ok: u64,
    total_done_fail: u64,
    last_error: String,
    #[serde(skip_serializing)]
    api_ms_samples: Vec<u32>,
    #[serde(skip_serializing)]
    eval_ms_samples: Vec<u32>,
    #[serde(skip_serializing)]
    done_history: Vec<(u64, bool)>,
    #[serde(skip_serializing)]
    digest_history: Vec<(u64, String)>,
    #[serde(skip_serializing)]
    root_cause_history: Vec<(u64, String)>,
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
    draft_exists: bool,
    resume: Option<ResumeInfo>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    service: &'static str,
    version: &'static str,
    unix_time: u64,
}

#[derive(Debug, Serialize)]
struct MetricsResponse {
    unix_time: u64,
    event_buffer_len: usize,
    next_event_id: usize,
    running: bool,
    total_events: u64,
    total_logs: u64,
    total_done_ok: u64,
    total_done_fail: u64,
    last_error: String,
    api_p50_ms: Option<u32>,
    api_p95_ms: Option<u32>,
    eval_p50_ms: Option<u32>,
    eval_p95_ms: Option<u32>,
    done_5m_ok: u64,
    done_5m_fail: u64,
    done_5m_success_rate: Option<f64>,
    readiness: String,
    readiness_score: u8,
    blockers: Vec<String>,
    actions: Vec<String>,
    gate_threshold: u8,
    gate_passed: bool,
    gate_reason: String,
    digest_5m: Vec<CategoryCount>,
    root_causes_5m: Vec<CategoryCount>,
    success_rate_series_5m: Vec<TimePoint>,
}

#[derive(Debug, Serialize)]
struct CategoryCount {
    category: String,
    count: u64,
}

#[derive(Debug, Serialize)]
struct TimePoint {
    minute_ago: u32,
    success_rate: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProjectConfig {
    auto_revert_profile: String,
    #[serde(default)]
    precheck_cmd: String,
    #[serde(default)]
    history_max_messages: String,
    #[serde(default)]
    history_max_chars: String,
    #[serde(default)]
    release_gate_threshold: String,
    updated_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WebProject {
    id: String,
    name: String,
    workspace: String,
    goal: String,
    updated_at: u64,
    snapshot: DraftPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct UiCachePayload {
    #[serde(default)]
    projects: Vec<WebProject>,
    #[serde(default)]
    shared_config: Option<SharedConfig>,
    #[serde(default)]
    project_logs: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    project_ui_state: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SharedConfig {
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct UiCachePatch {
    #[serde(default)]
    projects: Option<Vec<WebProject>>,
    #[serde(default)]
    shared_config: Option<SharedConfig>,
    #[serde(default)]
    project_logs: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    project_ui_state: Option<std::collections::BTreeMap<String, serde_json::Value>>,
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

fn normalize_rel_path(path: &Path) -> Option<PathBuf> {
    if path.is_absolute() {
        return None;
    }
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(seg) => out.push(seg),
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() {
        return None;
    }
    Some(out)
}

fn workspace_path_for_input(workspace: &str) -> Option<PathBuf> {
    let ws = workspace.trim();
    if ws.is_empty() {
        return None;
    }
    let rel = normalize_rel_path(Path::new(ws))?;
    let root_rel = normalize_rel_path(Path::new(MANAGED_ROOT_DIR))?;
    if !rel.starts_with(&root_rel) {
        return None;
    }
    Some(rel)
}

fn require_managed_workspace(workspace: &str) -> Result<PathBuf, String> {
    let rel = workspace_path_for_input(workspace).ok_or_else(|| {
        format!(
            "workspace must be under ./{}/{}",
            MANAGED_ROOT_DIR, MANAGED_WORKSPACES_DIR
        )
    })?;
    Ok(std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(rel))
}

fn read_draft(path: &Path) -> Option<DraftPayload> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn draft_path_for_workspace(workspace: &str) -> Option<PathBuf> {
    let full = require_managed_workspace(workspace).ok()?;
    Some(full.join(DRAFT_FILENAME))
}

fn save_draft(path: &Path, draft: &DraftPayload) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(draft)?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, json)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn save_project_config(
    workspace: &str,
    profile: &str,
    precheck_cmd: &str,
    history_max_messages: &str,
    history_max_chars: &str,
    release_gate_threshold: &str,
) -> std::io::Result<()> {
    let Ok(ws) = require_managed_workspace(workspace) else {
        return Ok(());
    };
    fs::create_dir_all(&ws)?;
    let cfg = ProjectConfig {
        auto_revert_profile: profile.to_string(),
        precheck_cmd: precheck_cmd.to_string(),
        history_max_messages: history_max_messages.to_string(),
        history_max_chars: history_max_chars.to_string(),
        release_gate_threshold: release_gate_threshold.to_string(),
        updated_at_unix: now_unix(),
    };
    let json = serde_json::to_string_pretty(&cfg)?;
    let path = ws.join(PROJECT_CONFIG_FILENAME);
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, json)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn read_project_config(workspace: &str) -> Option<ProjectConfig> {
    let ws = require_managed_workspace(workspace).ok()?;
    let path = ws.join(PROJECT_CONFIG_FILENAME);
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str::<ProjectConfig>(&content).ok()
}

fn ui_cache_path() -> PathBuf {
    managed_root_path().join(UI_CACHE_FILENAME)
}

fn read_ui_cache() -> UiCachePayload {
    let path = ui_cache_path();
    let content = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return UiCachePayload::default(),
    };
    serde_json::from_str::<UiCachePayload>(&content).unwrap_or_default()
}

fn write_ui_cache(payload: &UiCachePayload) -> std::io::Result<()> {
    let path = ui_cache_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(payload)?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, json)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn apply_runtime_config_envs(payload: &StartPayload) {
    set_or_clear_env("AUTOCODING_PRECHECK_CMD", &payload.precheck_cmd);
    set_or_clear_env(
        "AUTOCODING_HISTORY_MAX_MESSAGES",
        &payload.history_max_messages,
    );
    set_or_clear_env("AUTOCODING_HISTORY_MAX_CHARS", &payload.history_max_chars);
    set_or_clear_env(
        "AUTOCODING_RELEASE_GATE_THRESHOLD",
        &payload.release_gate_threshold,
    );
}

fn set_or_clear_env(key: &str, value: &str) {
    if value.trim().is_empty() {
        std::env::remove_var(key);
    } else {
        std::env::set_var(key, value.trim());
    }
}

fn read_resume_info(workspace: &str, goal: &str) -> Option<ResumeInfo> {
    let path = require_managed_workspace(workspace).ok()?.join(SESSION_STATE_FILENAME);
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
    apply_runtime_config_envs(&payload);
    let req = AgentRequest::Start {
        api_key: payload.api_key,
        base_url: payload.base_url,
        model: payload.model,
        auto_revert_profile: auto_revert_profile.clone(),
        resume_from_checkpoint,
        workspace: workspace_full.clone(),
        goal: payload.goal.clone(),
        eval_cmd: payload.eval_cmd,
        success_regex: payload.success_regex,
    };
    if let Err(e) = save_project_config(
        &payload.workspace,
        &auto_revert_profile,
        &payload.precheck_cmd,
        &payload.history_max_messages,
        &payload.history_max_chars,
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
    match start_from_payload(&data, payload, false) {
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

async fn save_draft_config(
    _data: web::Data<AppState>,
    body: web::Json<DraftPayload>,
) -> impl Responder {
    let payload = body.into_inner();
    let Some(draft_path) = draft_path_for_workspace(&payload.workspace) else {
        return HttpResponse::BadRequest().body("workspace is empty");
    };
    match save_draft(&draft_path, &payload) {
        Ok(_) => {
            if let Err(e) = save_project_config(
                &payload.workspace,
                &payload.auto_revert_profile,
                &payload.precheck_cmd,
                &payload.history_max_messages,
                &payload.history_max_chars,
                &payload.release_gate_threshold,
            ) {
                eprintln!("save project config during draft save failed: {}", e);
            }
            HttpResponse::Ok().body("saved")
        }
        Err(e) => HttpResponse::InternalServerError().body(format!("save draft failed: {}", e)),
    }
}

async fn get_draft_config(_data: web::Data<AppState>, query: web::Query<DraftQuery>) -> impl Responder {
    let Some(ws_path) = draft_path_for_workspace(&query.workspace) else {
        return HttpResponse::BadRequest().body("workspace is empty");
    };
    match read_draft(&ws_path) {
        Some(draft) => HttpResponse::Ok().json(draft),
        None => HttpResponse::NotFound().body("draft not found"),
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
    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    HttpResponse::Ok().json(items)
}

async fn upsert_project(data: web::Data<AppState>, body: web::Json<WebProject>) -> impl Responder {
    let _guard = data.projects_lock.lock().unwrap();
    let mut item = body.into_inner();
    if item.id.trim().is_empty() {
        return HttpResponse::BadRequest().body("project id is empty");
    }
    if item.name.trim().is_empty() {
        item.name = item.workspace.clone();
    }
    item.updated_at = now_unix();
    let mut cache = read_ui_cache();
    if let Some(idx) = cache.projects.iter().position(|p| p.id == item.id) {
        cache.projects[idx] = item.clone();
    } else {
        cache.projects.push(item.clone());
    }
    if let Err(e) = write_ui_cache(&cache) {
        return HttpResponse::InternalServerError().body(format!("write projects failed: {}", e));
    }
    HttpResponse::Ok().json(item)
}

async fn delete_project(data: web::Data<AppState>, path: web::Path<String>) -> impl Responder {
    let _guard = data.projects_lock.lock().unwrap();
    let id = path.into_inner();
    if id.trim().is_empty() {
        return HttpResponse::BadRequest().body("project id is empty");
    }
    let mut cache = read_ui_cache();
    let before = cache.projects.len();
    cache.projects.retain(|p| p.id != id);
    if cache.projects.len() == before {
        return HttpResponse::NotFound().body("project not found");
    }
    if let Err(e) = write_ui_cache(&cache) {
        return HttpResponse::InternalServerError().body(format!("write projects failed: {}", e));
    }
    HttpResponse::Ok().body("deleted")
}

async fn get_ui_cache(data: web::Data<AppState>) -> impl Responder {
    let _guard = data.projects_lock.lock().unwrap();
    HttpResponse::Ok().json(read_ui_cache())
}

async fn put_ui_cache(
    data: web::Data<AppState>,
    body: web::Json<UiCachePatch>,
) -> impl Responder {
    let _guard = data.projects_lock.lock().unwrap();
    let patch = body.into_inner();
    let mut payload = read_ui_cache();
    if let Some(v) = patch.projects {
        payload.projects = v;
    }
    if let Some(v) = patch.shared_config {
        payload.shared_config = Some(v);
    }
    if let Some(v) = patch.project_logs {
        payload.project_logs = v;
    }
    if let Some(v) = patch.project_ui_state {
        payload.project_ui_state = v;
    }
    match write_ui_cache(&payload) {
        Ok(_) => HttpResponse::Ok().body("saved"),
        Err(e) => HttpResponse::InternalServerError().body(format!("write ui cache failed: {}", e)),
    }
}

fn normalize_slug(raw: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in raw.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if matches!(c, '-' | '_' | ' ' | '\t' | '\n' | '\r') {
            if !prev_dash && !out.is_empty() {
                out.push('-');
                prev_dash = true;
            }
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.len() > 36 {
        out.truncate(36);
        while out.ends_with('-') {
            out.pop();
        }
    }
    out
}

fn fallback_slug(name: &str, goal: &str) -> String {
    let joined = format!("{} {}", name, goal);
    let s = normalize_slug(&joined);
    if s.is_empty() {
        format!("project-{}", now_unix())
    } else {
        s
    }
}

async fn suggest_project_slug(body: web::Json<SlugSuggestPayload>) -> impl Responder {
    let payload = body.into_inner();
    let fallback = fallback_slug(&payload.project_name, &payload.goal);
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
            let s = normalize_slug(text.trim());
            if s.is_empty() { fallback.clone() } else { s }
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
    let draft: DraftPayload = if !payload.project_id.trim().is_empty() {
        let _guard = data.projects_lock.lock().unwrap();
        let cache = read_ui_cache();
        let Some(project) = cache.projects.into_iter().find(|p| p.id == payload.project_id) else {
            return HttpResponse::NotFound().body("project not found");
        };
        project.snapshot
    } else {
        let Some(draft_path) = draft_path_for_workspace(&payload.workspace) else {
            return HttpResponse::BadRequest().body("workspace is empty");
        };
        let Some(d) = read_draft(&draft_path) else {
            return HttpResponse::NotFound().body("draft not found");
        };
        d
    };
    if draft.api_key.trim().is_empty() {
        return HttpResponse::BadRequest().body("api_key is empty in snapshot");
    }
    let resume = read_resume_info(&draft.workspace, &draft.goal);
    if resume.as_ref().map(|x| x.resumable).unwrap_or(false) {
        match start_from_payload(&data, draft.into_start(), true) {
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
    let draft = draft_path_for_workspace(&runtime.last_workspace).and_then(|p| read_draft(&p));
    let resume = draft
        .as_ref()
        .and_then(|d| read_resume_info(&d.workspace, &d.goal));
    HttpResponse::Ok().json(UiStateResponse {
        runtime,
        draft_exists: draft.is_some(),
        resume,
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
    let runtime = data.runtime.lock().unwrap().clone();
    let events_len = data.events.lock().unwrap().len();
    let next_id = *data.next_event_id.lock().unwrap();
    let (done_5m_ok, done_5m_fail) = done_recent_counts(&runtime.done_history, 300);
    let done_total = done_5m_ok + done_5m_fail;
    let done_5m_success_rate = if done_total == 0 {
        None
    } else {
        Some((done_5m_ok as f64) * 100.0 / (done_total as f64))
    };
    let readiness = readiness_level(&runtime, done_5m_ok, done_5m_fail).to_string();
    let readiness_score = readiness_score(
        &runtime,
        done_5m_ok,
        done_5m_fail,
        done_5m_success_rate,
        percentile_ms(&runtime.api_ms_samples, 95),
        percentile_ms(&runtime.eval_ms_samples, 95),
    );
    let blockers = release_blockers(
        &runtime,
        done_5m_fail,
        done_5m_success_rate,
        percentile_ms(&runtime.eval_ms_samples, 95),
    );
    let actions = release_actions(
        &runtime,
        percentile_ms(&runtime.api_ms_samples, 95),
        percentile_ms(&runtime.eval_ms_samples, 95),
    );
    let digest_5m = digest_recent_counts(&runtime.digest_history, 300);
    let root_causes_5m = digest_recent_counts(&runtime.root_cause_history, 300);
    let success_rate_series_5m = success_rate_series(&runtime.done_history, 5, 60);
    let gate_threshold = read_gate_threshold();
    let (gate_passed, gate_reason) =
        release_gate(readiness_score, gate_threshold, &blockers, runtime.running);
    let api_p50 = percentile_ms(&runtime.api_ms_samples, 50);
    let api_p95 = percentile_ms(&runtime.api_ms_samples, 95);
    let eval_p50 = percentile_ms(&runtime.eval_ms_samples, 50);
    let eval_p95 = percentile_ms(&runtime.eval_ms_samples, 95);
    HttpResponse::Ok().json(MetricsResponse {
        unix_time: now_unix(),
        event_buffer_len: events_len,
        next_event_id: next_id,
        running: runtime.running,
        total_events: runtime.total_events,
        total_logs: runtime.total_logs,
        total_done_ok: runtime.total_done_ok,
        total_done_fail: runtime.total_done_fail,
        last_error: runtime.last_error,
        api_p50_ms: api_p50,
        api_p95_ms: api_p95,
        eval_p50_ms: eval_p50,
        eval_p95_ms: eval_p95,
        done_5m_ok,
        done_5m_fail,
        done_5m_success_rate,
        readiness,
        readiness_score,
        blockers,
        actions,
        gate_threshold,
        gate_passed,
        gate_reason,
        digest_5m,
        root_causes_5m,
        success_rate_series_5m,
    })
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
    let list: Vec<_> = events
        .iter()
        .filter(|(id, _)| *id >= from_id)
        .cloned()
        .collect();
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
                events
                    .iter()
                    .filter(|(id, _)| *id >= next_id)
                    .cloned()
                    .collect()
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

async fn copy_zh_cn() -> impl Responder {
    HttpResponse::Ok()
        .content_type("application/json; charset=utf-8")
        .body(COPY_ZH_CN_JSON)
}

fn spawn_event_collector(state: AppState) {
    std::thread::spawn(move || {
        for evt in state.rx_evt.iter() {
            if let AgentEvent::Log(line) = &evt {
                println!("{}", line);
            }
            {
                let mut runtime = state.runtime.lock().unwrap();
                runtime.total_events += 1;
                match &evt {
                    AgentEvent::Log(line) => {
                        runtime.total_logs += 1;
                        if line.contains("失败") || line.to_lowercase().contains("error") {
                            runtime.last_error = line.clone();
                        }
                        if let Some(cat) = parse_eval_digest_category(line) {
                            push_digest(&mut runtime.digest_history, now_unix(), cat, 600);
                        }
                        if let Some(sig) = parse_eval_digest_signature(line) {
                            push_digest(&mut runtime.root_cause_history, now_unix(), sig, 600);
                        }
                        if let Some(ms) = parse_perf_ms(line, "[Perf] deepseek_api=") {
                            push_sample(&mut runtime.api_ms_samples, ms, 400);
                        }
                        if let Some(ms) = parse_eval_ms(line) {
                            push_sample(&mut runtime.eval_ms_samples, ms, 400);
                        }
                    }
                    AgentEvent::Done { success, message } => {
                        runtime.running = false;
                        push_done_history(&mut runtime.done_history, now_unix(), *success, 500);
                        if *success {
                            runtime.total_done_ok += 1;
                        } else {
                            runtime.total_done_fail += 1;
                            runtime.last_error = message.clone();
                        }
                    }
                    _ => {}
                }
            }
            let serial: SerializableEvent = evt.clone().into();
            let mut evts = state.events.lock().unwrap();
            let mut next_id = state.next_event_id.lock().unwrap();
            evts.push((*next_id, serial));
            if evts.len() > MAX_EVENT_BUFFER {
                let drop_n = evts.len() - MAX_EVENT_BUFFER;
                evts.drain(0..drop_n);
            }
            *next_id += 1;
        }
        // If event channel closes unexpectedly while UI still thinks it's running,
        // force a terminal event so the WebUI can converge to a stopped state.
        let should_emit_done = {
            let mut runtime = state.runtime.lock().unwrap();
            if runtime.running {
                runtime.running = false;
                if runtime.last_error.trim().is_empty() {
                    runtime.last_error = "agent event channel closed".to_string();
                }
                true
            } else {
                false
            }
        };
        if should_emit_done {
            let mut evts = state.events.lock().unwrap();
            let mut next_id = state.next_event_id.lock().unwrap();
            evts.push((
                *next_id,
                SerializableEvent::Done {
                    success: false,
                    message: "Agent event channel closed".to_string(),
                },
            ));
            if evts.len() > MAX_EVENT_BUFFER {
                let drop_n = evts.len() - MAX_EVENT_BUFFER;
                evts.drain(0..drop_n);
            }
            *next_id += 1;
        }
    });
}

fn parse_perf_ms(line: &str, prefix: &str) -> Option<u32> {
    let tail = line.strip_prefix(prefix)?;
    let n = tail.strip_suffix("ms")?;
    n.trim().parse::<u32>().ok()
}

fn parse_eval_ms(line: &str) -> Option<u32> {
    if !line.starts_with("[Perf] eval_") {
        return None;
    }
    let idx = line.find('=')?;
    let tail = &line[idx + 1..];
    let n = tail.strip_suffix("ms")?;
    n.trim().parse::<u32>().ok()
}

fn push_sample(samples: &mut Vec<u32>, value: u32, max_len: usize) {
    samples.push(value);
    if samples.len() > max_len {
        let drop_n = samples.len() - max_len;
        samples.drain(0..drop_n);
    }
}

fn percentile_ms(samples: &[u32], p: usize) -> Option<u32> {
    if samples.is_empty() || p == 0 {
        return None;
    }
    let mut v = samples.to_vec();
    v.sort_unstable();
    let idx = ((v.len() - 1) * p.min(100)) / 100;
    v.get(idx).copied()
}

fn parse_eval_digest_category(line: &str) -> Option<String> {
    let prefix = "[Eval-Digest] category=";
    let tail = line.strip_prefix(prefix)?;
    let category = tail.split_whitespace().next()?.trim();
    if category.is_empty() {
        return None;
    }
    Some(category.to_string())
}

fn parse_eval_digest_signature(line: &str) -> Option<String> {
    let key = " signature=";
    let idx = line.find(key)?;
    let sig = line[idx + key.len()..].trim();
    if sig.is_empty() {
        return None;
    }
    Some(sig.chars().take(120).collect::<String>())
}

fn push_digest(history: &mut Vec<(u64, String)>, ts: u64, category: String, max_len: usize) {
    history.push((ts, category));
    if history.len() > max_len {
        let drop_n = history.len() - max_len;
        history.drain(0..drop_n);
    }
}

fn digest_recent_counts(history: &[(u64, String)], window_secs: u64) -> Vec<CategoryCount> {
    let now = now_unix();
    let mut map: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    for (ts, cat) in history {
        if now.saturating_sub(*ts) <= window_secs {
            *map.entry(cat.clone()).or_insert(0) += 1;
        }
    }
    let mut out = map
        .into_iter()
        .map(|(category, count)| CategoryCount { category, count })
        .collect::<Vec<_>>();
    out.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.category.cmp(&b.category))
    });
    out.truncate(8);
    out
}

fn push_done_history(history: &mut Vec<(u64, bool)>, ts: u64, ok: bool, max_len: usize) {
    history.push((ts, ok));
    if history.len() > max_len {
        let drop_n = history.len() - max_len;
        history.drain(0..drop_n);
    }
}

fn done_recent_counts(history: &[(u64, bool)], window_secs: u64) -> (u64, u64) {
    let now = now_unix();
    let mut ok = 0u64;
    let mut fail = 0u64;
    for (ts, is_ok) in history.iter().copied() {
        if now.saturating_sub(ts) <= window_secs {
            if is_ok {
                ok += 1;
            } else {
                fail += 1;
            }
        }
    }
    (ok, fail)
}

fn readiness_level(runtime: &RuntimeStatus, done_5m_ok: u64, done_5m_fail: u64) -> &'static str {
    if runtime.running {
        return "running";
    }
    if done_5m_fail > 0 {
        return "red";
    }
    if done_5m_ok > 0 {
        return "green";
    }
    if runtime.total_done_fail > runtime.total_done_ok {
        return "yellow";
    }
    "idle"
}

fn readiness_score(
    runtime: &RuntimeStatus,
    done_5m_ok: u64,
    done_5m_fail: u64,
    done_5m_success_rate: Option<f64>,
    api_p95_ms: Option<u32>,
    eval_p95_ms: Option<u32>,
) -> u8 {
    let mut score: i32 = 100;
    if runtime.running {
        score -= 10;
    }
    if done_5m_fail > 0 {
        score -= 30;
    }
    if let Some(rate) = done_5m_success_rate {
        if rate < 60.0 {
            score -= 25;
        } else if rate < 85.0 {
            score -= 10;
        }
    }
    if done_5m_ok == 0 && done_5m_fail == 0 {
        score -= 5;
    }
    if let Some(v) = api_p95_ms {
        if v > 60_000 {
            score -= 10;
        }
    }
    if let Some(v) = eval_p95_ms {
        if v > 120_000 {
            score -= 15;
        }
    }
    if !runtime.last_error.trim().is_empty() {
        score -= 5;
    }
    score.clamp(0, 100) as u8
}

fn release_blockers(
    runtime: &RuntimeStatus,
    done_5m_fail: u64,
    done_5m_success_rate: Option<f64>,
    eval_p95_ms: Option<u32>,
) -> Vec<String> {
    let mut out = Vec::new();
    if done_5m_fail > 0 {
        out.push("最近5分钟存在失败结果".to_string());
    }
    if let Some(rate) = done_5m_success_rate {
        if rate < 80.0 {
            out.push(format!("最近5分钟成功率偏低: {:.1}%", rate));
        }
    }
    if let Some(v) = eval_p95_ms {
        if v > 120_000 {
            out.push(format!("评测耗时 p95 过高: {}ms", v));
        }
    }
    if runtime.last_error.to_lowercase().contains("session error") {
        out.push("最近会话出现 Session error".to_string());
    }
    out
}

fn release_actions(
    runtime: &RuntimeStatus,
    api_p95_ms: Option<u32>,
    eval_p95_ms: Option<u32>,
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(v) = api_p95_ms {
        if v > 60_000 {
            out.push("检查网络质量，或提高 AUTOCODING_API_TIMEOUT_SECS".to_string());
        }
    }
    if let Some(v) = eval_p95_ms {
        if v > 120_000 {
            out.push("检查评测脚本性能，必要时提高 AUTOCODING_EVAL_TIMEOUT_SECS".to_string());
        }
    }
    if !runtime.last_error.trim().is_empty() {
        out.push("根据 last_error 优先修复根因，再继续迭代".to_string());
    }
    if out.is_empty() {
        out.push("当前无明显阻断，可进行灰度发布".to_string());
    }
    out
}

fn success_rate_series(history: &[(u64, bool)], minutes: u32, bucket_secs: u64) -> Vec<TimePoint> {
    let now = now_unix();
    let mut out = Vec::new();
    for i in (0..minutes).rev() {
        let start = now.saturating_sub((i as u64 + 1) * bucket_secs);
        let end = now.saturating_sub((i as u64) * bucket_secs);
        let mut ok = 0u64;
        let mut total = 0u64;
        for (ts, is_ok) in history.iter().copied() {
            if ts >= start && ts < end {
                total += 1;
                if is_ok {
                    ok += 1;
                }
            }
        }
        let success_rate = if total == 0 {
            None
        } else {
            Some((ok as f64) * 100.0 / (total as f64))
        };
        out.push(TimePoint {
            minute_ago: i + 1,
            success_rate,
        });
    }
    out
}

fn read_gate_threshold() -> u8 {
    std::env::var("AUTOCODING_RELEASE_GATE_THRESHOLD")
        .ok()
        .and_then(|v| v.parse::<u8>().ok())
        .filter(|v| *v <= 100)
        .unwrap_or(75)
}

fn release_gate(
    readiness_score: u8,
    gate_threshold: u8,
    blockers: &[String],
    running: bool,
) -> (bool, String) {
    if running {
        return (false, "任务仍在运行中".to_string());
    }
    if !blockers.is_empty() {
        return (false, blockers.join("；"));
    }
    if readiness_score < gate_threshold {
        return (
            false,
            format!(
                "readiness_score={} 低于阈值 {}",
                readiness_score, gate_threshold
            ),
        );
    }
    (true, "通过发布门禁".to_string())
}

pub async fn run_web_server(
    tx_req: Sender<AgentRequest>,
    rx_evt: Receiver<AgentEvent>,
) -> std::io::Result<()> {
    fs::create_dir_all(managed_root_path())?;
    fs::create_dir_all(managed_workspaces_path())?;
    let port = std::env::var("AUTOCODING_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(8080);
    let state = AppState {
        tx_req,
        rx_evt,
        events: Arc::new(Mutex::new(Vec::new())),
        next_event_id: Arc::new(Mutex::new(0)),
        runtime: Arc::new(Mutex::new(RuntimeStatus::default())),
        projects_lock: Arc::new(Mutex::new(())),
    };
    spawn_event_collector(state.clone());

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(state.clone()))
            .route("/", web::get().to(index_page))
            .route("/assets/copy.zh-CN.json", web::get().to(copy_zh_cn))
            .route("/start", web::post().to(start_session))
            .route("/stop", web::post().to(stop_session))
            .route("/resume", web::post().to(resume_session))
            .route("/draft", web::post().to(save_draft_config))
            .route("/draft", web::get().to(get_draft_config))
            .route("/project_config", web::get().to(get_project_config))
            .route("/projects", web::get().to(list_projects))
            .route("/projects", web::post().to(upsert_project))
            .route("/projects/{id}", web::delete().to(delete_project))
            .route("/projects/suggest_slug", web::post().to(suggest_project_slug))
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
    use super::{readiness_score, release_gate};

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
        let score = readiness_score(
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
