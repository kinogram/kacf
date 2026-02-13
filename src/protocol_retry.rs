use anyhow::Error;
use std::time::Duration;

pub(crate) fn should_retry_session_error(err: &Error) -> bool {
    let s = err.to_string().to_lowercase();
    s.contains("timed out")
        || s.contains("timeout")
        || s.contains("read streaming chunk")
        || s.contains("connection reset")
        || s.contains("connection refused")
        || s.contains("temporarily unavailable")
        || s.contains("broken pipe")
        || s.contains("http status not success: 408")
        || s.contains("http status not success: 429")
        || s.contains("http status not success: 502")
        || s.contains("http status not success: 503")
        || s.contains("http status not success: 504")
}

pub(crate) fn session_retry_max_attempts() -> u32 {
    std::env::var("AUTOCODING_SESSION_RETRY_MAX")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| *v <= 8)
        .unwrap_or(2)
}

pub(crate) fn session_retry_delay(attempt: u32) -> Duration {
    let base_ms = std::env::var("AUTOCODING_SESSION_RETRY_BASE_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v >= 200 && *v <= 10_000)
        .unwrap_or(1_200);
    let factor = attempt.max(1) as u64;
    Duration::from_millis(base_ms.saturating_mul(factor))
}

#[cfg(test)]
mod tests {
    use super::{session_retry_delay, session_retry_max_attempts, should_retry_session_error};
    use anyhow::anyhow;
    use std::time::Duration;

    #[test]
    fn retry_classifier_matches_timeout() {
        let err = anyhow!("deepseek chat_complete: read streaming chunk: operation timed out");
        assert!(should_retry_session_error(&err));
    }

    #[test]
    fn retry_classifier_ignores_logic_errors() {
        let err = anyhow!("error: the package 'snake_game' does not contain this feature");
        assert!(!should_retry_session_error(&err));
    }

    #[test]
    fn retry_defaults_are_reasonable() {
        std::env::remove_var("AUTOCODING_SESSION_RETRY_MAX");
        std::env::remove_var("AUTOCODING_SESSION_RETRY_BASE_MS");
        assert_eq!(session_retry_max_attempts(), 2);
        assert_eq!(session_retry_delay(1), Duration::from_millis(1_200));
    }
}
