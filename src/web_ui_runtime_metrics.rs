pub(crate) fn build_metrics_response(
    runtime: &crate::web_ui::RuntimeStatus,
    events_len: usize,
    next_id: usize,
    now_unix: u64,
) -> crate::web_ui::MetricsResponse {
    let (done_5m_ok, done_5m_fail) =
        crate::web_ui_analytics::done_recent_counts(&runtime.done_history, 300);
    let done_total = done_5m_ok + done_5m_fail;
    let done_5m_success_rate = if done_total == 0 {
        None
    } else {
        Some((done_5m_ok as f64) * 100.0 / (done_total as f64))
    };
    let api_p50 = crate::web_ui_analytics::percentile_ms(&runtime.api_ms_samples, 50);
    let api_p95 = crate::web_ui_analytics::percentile_ms(&runtime.api_ms_samples, 95);
    let eval_p50 = crate::web_ui_analytics::percentile_ms(&runtime.eval_ms_samples, 50);
    let eval_p95 = crate::web_ui_analytics::percentile_ms(&runtime.eval_ms_samples, 95);
    let readiness =
        crate::web_ui_analytics::readiness_level(runtime, done_5m_ok, done_5m_fail).to_string();
    let readiness_score = crate::web_ui_analytics::readiness_score(
        runtime,
        done_5m_ok,
        done_5m_fail,
        done_5m_success_rate,
        api_p95,
        eval_p95,
    );
    let blockers = crate::web_ui_analytics::release_blockers(
        runtime,
        done_5m_fail,
        done_5m_success_rate,
        eval_p95,
    );
    let actions = crate::web_ui_analytics::release_actions(runtime, api_p95, eval_p95);
    let digest_5m = crate::web_ui_analytics::digest_recent_counts(&runtime.digest_history, 300);
    let root_causes_5m =
        crate::web_ui_analytics::digest_recent_counts(&runtime.root_cause_history, 300);
    let success_rate_series_5m =
        crate::web_ui_analytics::success_rate_series(&runtime.done_history, 5, 60);
    let gate_threshold = crate::web_ui::read_gate_threshold();
    let (gate_passed, gate_reason) =
        crate::web_ui::release_gate(readiness_score, gate_threshold, &blockers, runtime.running);
    crate::web_ui::MetricsResponse {
        unix_time: now_unix,
        event_buffer_len: events_len,
        next_event_id: next_id,
        running: runtime.running,
        total_events: runtime.total_events,
        total_logs: runtime.total_logs,
        total_done_ok: runtime.total_done_ok,
        total_done_fail: runtime.total_done_fail,
        last_error: runtime.last_error.clone(),
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
    }
}
