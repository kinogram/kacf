use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    #[serde(default)]
    pub(crate) git_user_name: String,
    #[serde(default)]
    pub(crate) git_user_email: String,
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
    #[serde(default)]
    pub(crate) git_user_name: String,
    #[serde(default)]
    pub(crate) git_user_email: String,
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
            git_user_name: self.git_user_name,
            git_user_email: self.git_user_email,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct VmInstance {
    pub(crate) name: String,
    pub(crate) backend: String,
    pub(crate) power_state: String,
    pub(crate) cpu: u32,
    pub(crate) memory_mb: u32,
    pub(crate) disk_gb: u32,
    pub(crate) disk_path: String,
    #[serde(default)]
    pub(crate) os_image: String,
    pub(crate) created_at_unix: u64,
    pub(crate) updated_at_unix: u64,
    #[serde(default)]
    pub(crate) last_message: String,
    #[serde(default)]
    pub(crate) process_id: Option<u32>,
    #[serde(default)]
    pub(crate) ssh_port: Option<u16>,
    #[serde(default)]
    pub(crate) ssh_user: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct VmStateStore {
    #[serde(default)]
    pub(crate) vms: BTreeMap<String, VmInstance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct VmCapability {
    pub(crate) backend: String,
    pub(crate) available: bool,
    pub(crate) detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct VmStatusResponse {
    pub(crate) readonly: bool,
    pub(crate) capabilities: Vec<VmCapability>,
    pub(crate) vms: Vec<VmInstance>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmProvisionPayload {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) backend: String,
    #[serde(default)]
    pub(crate) cpu: u32,
    #[serde(default)]
    pub(crate) memory_mb: u32,
    #[serde(default)]
    pub(crate) disk_gb: u32,
    #[serde(default)]
    pub(crate) os_image: String,
    #[serde(default)]
    pub(crate) ssh_user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmActionPayload {
    pub(crate) name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmDeletePayload {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) purge_disk: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmActionResponse {
    pub(crate) ok: bool,
    pub(crate) effective: bool,
    pub(crate) message: String,
    pub(crate) vm: Option<VmInstance>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmLogQuery {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) tail: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmLogsResponse {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) bytes: usize,
    pub(crate) text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmSnapshotPayload {
    pub(crate) name: String,
    pub(crate) snapshot: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmSnapshotListQuery {
    pub(crate) name: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmSnapshotListResponse {
    pub(crate) name: String,
    pub(crate) snapshots: Vec<VmSnapshotEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmSnapshotEntry {
    pub(crate) tag: String,
    #[serde(default)]
    pub(crate) vm_size: String,
    #[serde(default)]
    pub(crate) created_at: String,
    #[serde(default)]
    pub(crate) vm_clock: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmClonePayload {
    pub(crate) source_name: String,
    pub(crate) new_name: String,
    #[serde(default)]
    pub(crate) snapshot: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmExecPayload {
    pub(crate) name: String,
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) timeout_sec: u64,
    #[serde(default)]
    pub(crate) wait_ready_sec: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmExecResponse {
    pub(crate) ok: bool,
    pub(crate) exit_code: i32,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmExecCancelPayload {
    pub(crate) name: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmExecCancelResponse {
    pub(crate) ok: bool,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmBootstrapPayload {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct VmExecQueueItem {
    pub(crate) id: String,
    pub(crate) command: String,
    pub(crate) status: String,
    pub(crate) created_at_unix: u64,
    #[serde(default)]
    pub(crate) timeout_sec: u64,
    #[serde(default)]
    pub(crate) wait_ready_sec: u64,
    #[serde(default)]
    pub(crate) priority: i32,
    #[serde(default)]
    pub(crate) retry_max: u32,
    #[serde(default)]
    pub(crate) retry_count: u32,
    #[serde(default)]
    pub(crate) next_run_after_unix: u64,
    #[serde(default)]
    pub(crate) started_at_unix: u64,
    #[serde(default)]
    pub(crate) finished_at_unix: u64,
    #[serde(default)]
    pub(crate) exit_code: i32,
    #[serde(default)]
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) output_preview: String,
    #[serde(default)]
    pub(crate) failure_category: String,
    #[serde(default)]
    pub(crate) failure_signature: String,
    #[serde(default)]
    pub(crate) failure_key_lines: Vec<String>,
    #[serde(default)]
    pub(crate) run_id: String,
    #[serde(default)]
    pub(crate) run_kind: String,
    #[serde(default)]
    pub(crate) run_max_runtime_sec: u64,
    #[serde(default)]
    pub(crate) command_risk_level: String,
    #[serde(default)]
    pub(crate) command_risk_tags: Vec<String>,
    #[serde(default)]
    pub(crate) strategy_signature: String,
    #[serde(default)]
    pub(crate) trigger_task_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmQueueQuery {
    pub(crate) name: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmQueueResponse {
    pub(crate) name: String,
    pub(crate) items: Vec<VmExecQueueItem>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmQueueStatsResponse {
    pub(crate) name: String,
    pub(crate) total: usize,
    pub(crate) pending: usize,
    pub(crate) running: usize,
    pub(crate) done: usize,
    pub(crate) failed: usize,
    pub(crate) canceled: usize,
    pub(crate) done_success_rate: f64,
    pub(crate) avg_duration_sec: f64,
    #[serde(default)]
    pub(crate) running_total_all_vms: usize,
    #[serde(default)]
    pub(crate) running_limit_all_vms: usize,
    #[serde(default)]
    pub(crate) watchdog_recovered_total: usize,
    #[serde(default)]
    pub(crate) watchdog_last_recovered_unix: u64,
    #[serde(default)]
    pub(crate) oldest_pending_age_sec: u64,
    #[serde(default)]
    pub(crate) top_pending_effective_priority: i32,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmExecDispatchResponse {
    pub(crate) started_workers: usize,
    pub(crate) running_total_all_vms: usize,
    pub(crate) running_limit_all_vms: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct VmExecDispatchTraceCandidate {
    pub(crate) vm_name: String,
    pub(crate) base_priority: i32,
    pub(crate) effective_priority: i32,
    pub(crate) oldest_pending_age_sec: u64,
    pub(crate) selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct VmExecDispatchTraceEntry {
    pub(crate) created_at_unix: u64,
    pub(crate) running_total_before: usize,
    pub(crate) running_limit: usize,
    pub(crate) available_slots: usize,
    pub(crate) selected_vms: Vec<String>,
    pub(crate) candidates: Vec<VmExecDispatchTraceCandidate>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmExecDispatchTraceQuery {
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmExecDispatchTraceResponse {
    pub(crate) entries: Vec<VmExecDispatchTraceEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmHealthScanPayload {
    #[serde(default)]
    pub(crate) self_heal: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmHealthIssue {
    pub(crate) vm_name: String,
    pub(crate) severity: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmHealthAction {
    pub(crate) vm_name: String,
    pub(crate) action: String,
    pub(crate) detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmHealthScanResponse {
    pub(crate) scanned: usize,
    pub(crate) issues: Vec<VmHealthIssue>,
    pub(crate) actions: Vec<VmHealthAction>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmExecEnqueuePayload {
    pub(crate) name: String,
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) timeout_sec: u64,
    #[serde(default)]
    pub(crate) wait_ready_sec: u64,
    #[serde(default)]
    pub(crate) priority: i32,
    #[serde(default)]
    pub(crate) retry_max: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmExecBatchTaskPayload {
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) timeout_sec: u64,
    #[serde(default)]
    pub(crate) wait_ready_sec: u64,
    #[serde(default)]
    pub(crate) priority: i32,
    #[serde(default)]
    pub(crate) retry_max: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmExecEnqueueBatchPayload {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) timeout_sec: u64,
    #[serde(default)]
    pub(crate) wait_ready_sec: u64,
    #[serde(default)]
    pub(crate) priority: i32,
    #[serde(default)]
    pub(crate) retry_max: u32,
    #[serde(default)]
    pub(crate) tasks: Vec<VmExecBatchTaskPayload>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmExecEnqueueProfilePayload {
    pub(crate) name: String,
    pub(crate) profile: String,
    #[serde(default)]
    pub(crate) workdir: String,
    #[serde(default)]
    pub(crate) test_cmd: String,
    #[serde(default)]
    pub(crate) timeout_sec: u64,
    #[serde(default)]
    pub(crate) wait_ready_sec: u64,
    #[serde(default)]
    pub(crate) priority: i32,
    #[serde(default)]
    pub(crate) retry_max: u32,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmExecProfilesResponse {
    pub(crate) profiles: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmExecProfileDetailQuery {
    pub(crate) profile: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmExecProfileDetailResponse {
    pub(crate) profile: String,
    pub(crate) commands: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmExecProfilePreviewQuery {
    pub(crate) profile: String,
    #[serde(default)]
    pub(crate) workdir: String,
    #[serde(default)]
    pub(crate) test_cmd: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmExecProfilePreviewResponse {
    pub(crate) profile: String,
    pub(crate) tasks: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmExecCustomProfileSavePayload {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) commands: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmExecCustomProfileDeletePayload {
    pub(crate) name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmSelfDebugPlanPayload {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) run_id: String,
    #[serde(default)]
    pub(crate) profile: String,
    #[serde(default)]
    pub(crate) cycles: u32,
    #[serde(default)]
    pub(crate) workdir: String,
    #[serde(default)]
    pub(crate) test_cmd: String,
    #[serde(default)]
    pub(crate) fix_cmd: String,
    #[serde(default)]
    pub(crate) verify_cmd: String,
    #[serde(default)]
    pub(crate) success_streak_target: u32,
    #[serde(default)]
    pub(crate) fail_streak_target: u32,
    #[serde(default)]
    pub(crate) max_task_budget: u32,
    #[serde(default)]
    pub(crate) max_runtime_sec: u64,
    #[serde(default)]
    pub(crate) timeout_sec: u64,
    #[serde(default)]
    pub(crate) wait_ready_sec: u64,
    #[serde(default)]
    pub(crate) priority: i32,
    #[serde(default)]
    pub(crate) retry_max: u32,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmSelfDebugPlanResponse {
    pub(crate) name: String,
    pub(crate) total_tasks: usize,
    pub(crate) tasks: Vec<String>,
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) run_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmSelfDebugRunsQuery {
    pub(crate) name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmSelfDebugStopPayload {
    pub(crate) name: String,
    pub(crate) run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct VmSelfDebugRunSummary {
    pub(crate) run_id: String,
    pub(crate) total: usize,
    pub(crate) pending: usize,
    pub(crate) paused: usize,
    pub(crate) running: usize,
    pub(crate) done: usize,
    pub(crate) failed: usize,
    pub(crate) canceled: usize,
    pub(crate) updated_at_unix: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmSelfDebugRunsResponse {
    pub(crate) name: String,
    pub(crate) runs: Vec<VmSelfDebugRunSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct VmSelfDebugHistoryEntry {
    pub(crate) summary: VmSelfDebugRunSummary,
    pub(crate) archived_at_unix: u64,
    #[serde(default)]
    pub(crate) failed_steps: usize,
    #[serde(default)]
    pub(crate) categories: Vec<String>,
    #[serde(default)]
    pub(crate) key_lines: Vec<String>,
    #[serde(default)]
    pub(crate) context_text: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmSelfDebugHistoryResponse {
    pub(crate) name: String,
    pub(crate) history: Vec<VmSelfDebugHistoryEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmSelfDebugHistoryDetailResponse {
    pub(crate) name: String,
    pub(crate) run_id: String,
    pub(crate) entry: VmSelfDebugHistoryEntry,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmSelfDebugHistoryArchivePayload {
    pub(crate) name: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmSelfDebugHistoryArchiveResponse {
    pub(crate) name: String,
    pub(crate) archived_runs: usize,
    pub(crate) removed_tasks: usize,
    pub(crate) history_total: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmSelfDebugHistoryClearPayload {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) run_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmSelfDebugHistoryClearResponse {
    pub(crate) name: String,
    pub(crate) removed: usize,
    pub(crate) history_total: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmSelfDebugStrategyStat {
    pub(crate) category: String,
    pub(crate) attempts: usize,
    pub(crate) verified_success: usize,
    pub(crate) verified_fail: usize,
    pub(crate) pending: usize,
    pub(crate) success_rate: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmSelfDebugStrategyStatsResponse {
    pub(crate) name: String,
    pub(crate) stats: Vec<VmSelfDebugStrategyStat>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmSelfDebugStrategyRulesResponse {
    pub(crate) rules: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmSelfDebugStrategyRulesSavePayload {
    #[serde(default)]
    pub(crate) rules: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmSelfDebugRunDetailQuery {
    pub(crate) name: String,
    pub(crate) run_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmSelfDebugRunTaskDetail {
    pub(crate) id: String,
    pub(crate) run_kind: String,
    pub(crate) status: String,
    pub(crate) exit_code: i32,
    pub(crate) command: String,
    pub(crate) message: String,
    pub(crate) output_preview: String,
    pub(crate) failure_category: String,
    pub(crate) failure_signature: String,
    pub(crate) failure_key_lines: Vec<String>,
    pub(crate) command_risk_level: String,
    pub(crate) command_risk_tags: Vec<String>,
    pub(crate) strategy_signature: String,
    pub(crate) trigger_task_id: String,
    pub(crate) created_at_unix: u64,
    pub(crate) started_at_unix: u64,
    pub(crate) finished_at_unix: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmSelfDebugRunDetailResponse {
    pub(crate) name: String,
    pub(crate) run_id: String,
    pub(crate) tasks: Vec<VmSelfDebugRunTaskDetail>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmSelfDebugContextResponse {
    pub(crate) name: String,
    pub(crate) run_id: String,
    pub(crate) failed_steps: usize,
    pub(crate) categories: Vec<String>,
    pub(crate) key_lines: Vec<String>,
    pub(crate) context_text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmQueueCancelPayload {
    pub(crate) name: String,
    pub(crate) task_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VmReadyQuery {
    pub(crate) name: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct VmReadyResponse {
    pub(crate) ok: bool,
    pub(crate) message: String,
}
