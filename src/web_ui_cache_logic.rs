const DEFAULT_LOG_MAX_CHARS: usize = 20_000;

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
    let log_max_chars = payload
        .global_options
        .as_ref()
        .map(|g| parse_limit(&g.log_max_chars, DEFAULT_LOG_MAX_CHARS))
        .unwrap_or(DEFAULT_LOG_MAX_CHARS);
    if let Some(v) = patch.project_logs {
        payload.project_logs = v
            .into_iter()
            .map(|(k, val)| (k, sanitize_log_bucket(&val, log_max_chars)))
            .collect();
    }
    if let Some(v) = patch.project_ui_state {
        payload.project_ui_state = v;
    }
}

#[cfg(test)]
mod tests {
    use super::apply_ui_cache_patch;
    use crate::web_ui::{GlobalOptions, UiCachePatch, UiCachePayload};
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
                    log_max_chars: "20000".to_string(),
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
        assert!(out.chars().count() <= 20_001);
    }
}
