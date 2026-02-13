pub(crate) fn apply_runtime_config_envs(
    precheck_cmd: &str,
    history_max_messages: &str,
    history_max_chars: &str,
    release_gate_threshold: &str,
    session_retry_max: &str,
    session_retry_base_ms: &str,
) {
    set_or_clear_env("AUTOCODING_PRECHECK_CMD", precheck_cmd);
    set_or_clear_env("AUTOCODING_HISTORY_MAX_MESSAGES", history_max_messages);
    set_or_clear_env("AUTOCODING_HISTORY_MAX_CHARS", history_max_chars);
    set_or_clear_env("AUTOCODING_RELEASE_GATE_THRESHOLD", release_gate_threshold);
    set_or_clear_env("AUTOCODING_SESSION_RETRY_MAX", session_retry_max);
    set_or_clear_env("AUTOCODING_SESSION_RETRY_BASE_MS", session_retry_base_ms);
}

fn set_or_clear_env(key: &str, value: &str) {
    if value.trim().is_empty() {
        std::env::remove_var(key);
    } else {
        std::env::set_var(key, value.trim());
    }
}

#[cfg(test)]
mod tests {
    use super::apply_runtime_config_envs;

    #[test]
    fn apply_runtime_config_sets_and_clears_envs() {
        apply_runtime_config_envs("cargo check", "40", "70000", "75", "3", "1500");
        assert_eq!(
            std::env::var("AUTOCODING_PRECHECK_CMD").ok().as_deref(),
            Some("cargo check")
        );
        assert_eq!(
            std::env::var("AUTOCODING_HISTORY_MAX_MESSAGES")
                .ok()
                .as_deref(),
            Some("40")
        );
        assert_eq!(
            std::env::var("AUTOCODING_HISTORY_MAX_CHARS")
                .ok()
                .as_deref(),
            Some("70000")
        );
        assert_eq!(
            std::env::var("AUTOCODING_RELEASE_GATE_THRESHOLD")
                .ok()
                .as_deref(),
            Some("75")
        );
        assert_eq!(
            std::env::var("AUTOCODING_SESSION_RETRY_MAX")
                .ok()
                .as_deref(),
            Some("3")
        );
        assert_eq!(
            std::env::var("AUTOCODING_SESSION_RETRY_BASE_MS")
                .ok()
                .as_deref(),
            Some("1500")
        );
        apply_runtime_config_envs("", "", "", "", "", "");
        assert!(std::env::var("AUTOCODING_PRECHECK_CMD").is_err());
        assert!(std::env::var("AUTOCODING_HISTORY_MAX_MESSAGES").is_err());
        assert!(std::env::var("AUTOCODING_HISTORY_MAX_CHARS").is_err());
        assert!(std::env::var("AUTOCODING_RELEASE_GATE_THRESHOLD").is_err());
        assert!(std::env::var("AUTOCODING_SESSION_RETRY_MAX").is_err());
        assert!(std::env::var("AUTOCODING_SESSION_RETRY_BASE_MS").is_err());
    }
}
