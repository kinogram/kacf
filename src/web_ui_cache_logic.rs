const MAX_LOG_CHARS_PER_BUCKET: usize = 120_000;
const MAX_LOG_LINES_PER_BUCKET: usize = 2_500;
const MAX_DIFF_CHARS: usize = 120_000;
const MAX_DIFF_LINES: usize = 2_500;

fn truncate_tail_chars(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let total = input.chars().count();
    if total <= max_chars {
        return input.to_string();
    }
    input.chars().skip(total - max_chars).collect()
}

fn keep_tail_lines(input: &str, max_lines: usize) -> String {
    if max_lines == 0 {
        return String::new();
    }
    let mut lines: Vec<&str> = input.lines().collect();
    if lines.len() > max_lines {
        lines = lines.split_off(lines.len() - max_lines);
    }
    lines.join("\n")
}

fn sanitize_log_bucket(raw: &str) -> String {
    let tail = truncate_tail_chars(raw, MAX_LOG_CHARS_PER_BUCKET);
    let trimmed_lines = keep_tail_lines(&tail, MAX_LOG_LINES_PER_BUCKET);
    if trimmed_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", trimmed_lines)
    }
}

fn sanitize_diff_text(raw: &str) -> String {
    let tail = truncate_tail_chars(raw, MAX_DIFF_CHARS);
    keep_tail_lines(&tail, MAX_DIFF_LINES)
}

pub(crate) fn apply_ui_cache_patch(
    payload: &mut crate::web_ui::UiCachePayload,
    patch: crate::web_ui::UiCachePatch,
) {
    if let Some(v) = patch.projects {
        payload.projects = v;
    }
    if let Some(v) = patch.shared_config {
        payload.shared_config = Some(v);
    }
    if let Some(v) = patch.global_options {
        payload.global_options = Some(v);
    }
    if let Some(v) = patch.project_logs {
        payload.project_logs = v
            .into_iter()
            .map(|(k, val)| (k, sanitize_log_bucket(&val)))
            .collect();
    }
    if let Some(v) = patch.project_ui_state {
        payload.project_ui_state = v
            .into_iter()
            .map(|(k, mut val)| {
                if let Some(obj) = val.as_object_mut() {
                    if let Some(raw) = obj.get("diff_text").and_then(|x| x.as_str()) {
                        obj.insert(
                            "diff_text".to_string(),
                            serde_json::Value::String(sanitize_diff_text(raw)),
                        );
                    }
                }
                (k, val)
            })
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::apply_ui_cache_patch;
    use crate::web_ui::{UiCachePatch, UiCachePayload};
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn patch_sanitizes_project_logs() {
        let mut payload = UiCachePayload::default();
        let mut logs = BTreeMap::new();
        logs.insert("project:a".to_string(), format!("{}\n", "x".repeat(140_000)));
        apply_ui_cache_patch(
            &mut payload,
            UiCachePatch {
                project_logs: Some(logs),
                ..UiCachePatch::default()
            },
        );
        let out = payload.project_logs.get("project:a").cloned().unwrap_or_default();
        assert!(out.chars().count() <= 120_001);
    }

    #[test]
    fn patch_sanitizes_diff_text_in_ui_state() {
        let mut payload = UiCachePayload::default();
        let mut ui_state = BTreeMap::new();
        ui_state.insert(
            "project:a".to_string(),
            json!({
                "status_text": "ok",
                "diff_text": "A".repeat(140_000),
            }),
        );
        apply_ui_cache_patch(
            &mut payload,
            UiCachePatch {
                project_ui_state: Some(ui_state),
                ..UiCachePatch::default()
            },
        );
        let out = payload
            .project_ui_state
            .get("project:a")
            .and_then(|v| v.get("diff_text"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(out.chars().count() <= 120_000);
    }
}
