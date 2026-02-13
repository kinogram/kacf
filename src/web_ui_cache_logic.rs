const DEFAULT_LOG_MAX_CHARS: usize = 50_000;
const DEFAULT_DIFF_MAX_CHARS: usize = 50_000;

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

fn parse_limit(raw: &str, fallback: usize) -> usize {
    raw.trim()
        .parse::<usize>()
        .ok()
        .filter(|v| *v > 0)
        .unwrap_or(fallback)
}

fn sanitize_log_bucket(raw: &str, max_chars: usize) -> String {
    let tail = truncate_tail_chars(raw, max_chars);
    if tail.is_empty() {
        String::new()
    } else {
        format!("{}\n", tail.trim_end_matches('\n'))
    }
}

fn sanitize_diff_text(raw: &str, max_chars: usize) -> String {
    truncate_tail_chars(raw, max_chars)
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
    let (log_max_chars, diff_max_chars) = payload
        .global_options
        .as_ref()
        .map(|g| {
            (
                parse_limit(&g.log_max_chars, DEFAULT_LOG_MAX_CHARS),
                parse_limit(&g.diff_max_chars, DEFAULT_DIFF_MAX_CHARS),
            )
        })
        .unwrap_or((DEFAULT_LOG_MAX_CHARS, DEFAULT_DIFF_MAX_CHARS));
    if let Some(v) = patch.project_logs {
        payload.project_logs = v
            .into_iter()
            .map(|(k, val)| (k, sanitize_log_bucket(&val, log_max_chars)))
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
                            serde_json::Value::String(sanitize_diff_text(raw, diff_max_chars)),
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
    use crate::web_ui::{GlobalOptions, UiCachePatch, UiCachePayload};
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn patch_sanitizes_project_logs() {
        let mut payload = UiCachePayload::default();
        let mut logs = BTreeMap::new();
        logs.insert(
            "project:a".to_string(),
            format!("{}\n", "x".repeat(140_000)),
        );
        apply_ui_cache_patch(
            &mut payload,
            UiCachePatch {
                global_options: Some(GlobalOptions {
                    log_max_chars: "50000".to_string(),
                    ..GlobalOptions::default()
                }),
                project_logs: Some(logs),
                ..UiCachePatch::default()
            },
        );
        let out = payload
            .project_logs
            .get("project:a")
            .cloned()
            .unwrap_or_default();
        assert!(out.chars().count() <= 50_001);
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
                global_options: Some(GlobalOptions {
                    diff_max_chars: "50000".to_string(),
                    ..GlobalOptions::default()
                }),
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
        assert!(out.chars().count() <= 50_000);
    }
}
