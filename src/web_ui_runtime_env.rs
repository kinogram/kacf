pub(crate) fn apply_runtime_config_envs(
    precheck_cmd: &str,
    release_gate_threshold: &str,
) {
    set_or_clear_env("AUTOCODING_PRECHECK_CMD", precheck_cmd);
    // Removed from WebUI project config; always clear to avoid stale inherited values.
    std::env::remove_var("AUTOCODING_HISTORY_MAX_MESSAGES");
    std::env::remove_var("AUTOCODING_HISTORY_MAX_CHARS");
    set_or_clear_env("AUTOCODING_RELEASE_GATE_THRESHOLD", release_gate_threshold);
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
        apply_runtime_config_envs("cargo check", "75");
        assert_eq!(
            std::env::var("AUTOCODING_PRECHECK_CMD").ok().as_deref(),
            Some("cargo check")
        );
        assert!(std::env::var("AUTOCODING_HISTORY_MAX_MESSAGES").is_err());
        assert!(std::env::var("AUTOCODING_HISTORY_MAX_CHARS").is_err());
        assert_eq!(
            std::env::var("AUTOCODING_RELEASE_GATE_THRESHOLD")
                .ok()
                .as_deref(),
            Some("75")
        );
        apply_runtime_config_envs("", "");
        assert!(std::env::var("AUTOCODING_PRECHECK_CMD").is_err());
        assert!(std::env::var("AUTOCODING_HISTORY_MAX_MESSAGES").is_err());
        assert!(std::env::var("AUTOCODING_HISTORY_MAX_CHARS").is_err());
        assert!(std::env::var("AUTOCODING_RELEASE_GATE_THRESHOLD").is_err());
    }
}
