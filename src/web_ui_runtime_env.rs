pub(crate) fn apply_runtime_config_envs(history_max_messages: &str, history_max_chars: &str) {
    set_or_clear_env("AUTOCODING_HISTORY_MAX_MESSAGES", history_max_messages);
    set_or_clear_env("AUTOCODING_HISTORY_MAX_CHARS", history_max_chars);
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
        apply_runtime_config_envs("40", "70000");
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
        apply_runtime_config_envs("", "");
        assert!(std::env::var("AUTOCODING_HISTORY_MAX_MESSAGES").is_err());
        assert!(std::env::var("AUTOCODING_HISTORY_MAX_CHARS").is_err());
    }
}
