use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StartPayload {
    pub(crate) api_key: String,
    pub(crate) base_url: String,
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) language: String,
    #[serde(default)]
    pub(crate) unattended_mode: bool,
    pub(crate) workspace: String,
    pub(crate) goal: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DraftPayload {
    pub(crate) api_key: String,
    pub(crate) base_url: String,
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) language: String,
    #[serde(default)]
    pub(crate) unattended_mode: bool,
    pub(crate) workspace: String,
    pub(crate) goal: String,
    pub(crate) remote: String,
    pub(crate) remote_url: String,
    pub(crate) branch: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProjectConfigQuery {
    pub(crate) workspace: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResumePayload {
    #[serde(default)]
    pub(crate) project_id: String,
    #[serde(default)]
    pub(crate) unattended_mode: bool,
}

impl DraftPayload {
    pub(crate) fn into_start(self) -> StartPayload {
        StartPayload {
            api_key: self.api_key,
            base_url: self.base_url,
            model: self.model,
            language: self.language,
            unattended_mode: self.unattended_mode,
            workspace: self.workspace,
            goal: self.goal,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ClarifyPayload {
    pub(crate) answers: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PushPayload {
    pub(crate) remote: String,
    pub(crate) url: String,
    pub(crate) branch: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SlugSuggestPayload {
    #[serde(default)]
    pub(crate) project_name: String,
    #[serde(default)]
    pub(crate) goal: String,
    #[serde(default)]
    pub(crate) api_key: String,
    #[serde(default = "default_base_url")]
    pub(crate) base_url: String,
    #[serde(default = "default_model_name")]
    pub(crate) model: String,
}

pub(crate) fn default_base_url() -> String {
    "https://api.deepseek.com".to_string()
}

pub(crate) fn default_model_name() -> String {
    "deepseek-reasoner".to_string()
}

#[derive(Debug, Serialize)]
pub(crate) struct SlugSuggestResponse {
    pub(crate) slug: String,
    pub(crate) source: String,
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
pub(crate) struct ResumeInfo {
    pub(crate) resumable: bool,
    pub(crate) updated_at_unix: Option<u64>,
    pub(crate) iteration: Option<u32>,
    pub(crate) message_count: Option<usize>,
    pub(crate) last_status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResumeMeta {
    #[serde(default)]
    pub(crate) goal: String,
    #[serde(default)]
    pub(crate) updated_at_unix: Option<u64>,
    #[serde(default)]
    pub(crate) iteration: Option<u32>,
    #[serde(default)]
    pub(crate) message_count: Option<usize>,
    #[serde(default)]
    pub(crate) last_status: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct UiStateResponse {
    pub(crate) runtime: RuntimeStatus,
    pub(crate) resume: Option<ResumeInfo>,
}

#[derive(Debug, Serialize)]
pub(crate) struct LanguageListResponse {
    pub(crate) languages: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct HealthResponse {
    pub(crate) ok: bool,
    pub(crate) service: &'static str,
    pub(crate) version: &'static str,
    pub(crate) unix_time: u64,
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
    pub(crate) digest_5m: Vec<crate::web_ui_analytics::CategoryCount>,
    pub(crate) root_causes_5m: Vec<crate::web_ui_analytics::CategoryCount>,
    pub(crate) success_rate_series_5m: Vec<crate::web_ui_analytics::TimePoint>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ProjectConfig {
    #[serde(default)]
    pub(crate) unattended_mode: bool,
    pub(crate) updated_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WebProject {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) workspace: String,
    pub(crate) goal: String,
    pub(crate) updated_at: u64,
    pub(crate) snapshot: DraftPayload,
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
