use std::fs;
use std::path::Path;

const SESSION_STATE_FILENAME: &str = ".autocoding_state.json";

pub(crate) fn load_session_state(workspace: &Path) -> Option<crate::protocol::SessionState> {
    let state_path = workspace.join(SESSION_STATE_FILENAME);
    match fs::read_to_string(&state_path) {
        Ok(content) => serde_json::from_str::<crate::protocol::SessionState>(&content).ok(),
        Err(_) => None,
    }
}

pub(crate) fn save_session_state(workspace: &Path, state: &crate::protocol::SessionState) {
    let state_path = workspace.join(SESSION_STATE_FILENAME);
    if let Ok(json) = serde_json::to_string_pretty(state) {
        // Write to a temporary file then rename for atomicity.
        let tmp_path = state_path.with_extension("tmp");
        if fs::write(&tmp_path, json).is_ok() {
            let _ = fs::rename(tmp_path, state_path);
        }
    }
}

pub(crate) fn now_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{load_session_state, now_unix};
    use std::path::Path;

    #[test]
    fn load_session_state_returns_none_when_absent() {
        let p = Path::new("/tmp/kacf_non_existing_workspace_for_test");
        assert!(load_session_state(p).is_none());
    }

    #[test]
    fn now_unix_is_non_zero_for_modern_systems() {
        assert!(now_unix() > 1_600_000_000);
    }
}
