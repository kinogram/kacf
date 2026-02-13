pub(crate) fn trim_message_history_for_speed(
    messages: &mut Vec<crate::deepseek_api::ChatMessage>,
) -> Option<(usize, usize)> {
    let max_messages = read_history_max_messages();
    let max_chars = read_history_max_chars();
    if messages.is_empty() {
        return None;
    }
    let total_chars: usize = messages.iter().map(|m| m.content.len()).sum();
    if messages.len() <= max_messages && total_chars <= max_chars {
        return None;
    }

    let mut kept = Vec::new();
    let mut used_chars = 0usize;
    let mut start_idx = 0usize;

    if messages[0].role == "system" {
        used_chars += messages[0].content.len();
        kept.push(messages[0].clone());
        start_idx = 1;
    }

    let mut tail = Vec::new();
    for idx in (start_idx..messages.len()).rev() {
        let m = &messages[idx];
        let next_count = kept.len() + tail.len() + 1;
        let next_chars = used_chars + m.content.len();
        if next_count > max_messages || next_chars > max_chars {
            break;
        }
        tail.push(m.clone());
        used_chars = next_chars;
    }
    tail.reverse();
    kept.extend(tail);

    let removed_msgs = messages.len().saturating_sub(kept.len());
    let removed_chars = total_chars.saturating_sub(used_chars);
    if removed_msgs == 0 {
        return None;
    }
    *messages = kept;
    Some((removed_msgs, removed_chars))
}

pub(crate) fn compact_assistant_text(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    format!(
        "{}\n...[history truncated {} chars]",
        &s[..max],
        s.len() - max
    )
}

pub(crate) fn compact_patch_history(
    summary: &str,
    files: &[crate::protocol::FileWrite],
    raw_patch_json: &str,
) -> String {
    let paths = files
        .iter()
        .take(30)
        .map(|f| f.path.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let header = format!(
        "kind=patch summary={} file_count={} paths=[{}]",
        summary,
        files.len(),
        paths
    );
    let raw = compact_assistant_text(raw_patch_json, 16_000);
    format!("{header}\n{raw}")
}

fn read_history_max_messages() -> usize {
    std::env::var("AUTOCODING_HISTORY_MAX_MESSAGES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v >= 10 && *v <= 200)
        .unwrap_or(40)
}

fn read_history_max_chars() -> usize {
    std::env::var("AUTOCODING_HISTORY_MAX_CHARS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v >= 10_000 && *v <= 2_000_000)
        .unwrap_or(70_000)
}

#[cfg(test)]
mod tests {
    use super::{compact_assistant_text, compact_patch_history, trim_message_history_for_speed};
    use crate::deepseek_api::ChatMessage;
    use crate::protocol::FileWrite;

    #[test]
    fn compact_assistant_text_truncates_and_marks() {
        let s = "a".repeat(100);
        let out = compact_assistant_text(&s, 20);
        assert!(out.contains("truncated"));
        assert!(out.len() < s.len() + 80);
    }

    #[test]
    fn compact_patch_history_includes_header_and_paths() {
        let files = vec![FileWrite {
            path: "src/main.rs".to_string(),
            content: "fn main() {}".to_string(),
        }];
        let out = compact_patch_history("fix", &files, "{\"kind\":\"patch\"}");
        assert!(out.contains("kind=patch summary=fix"));
        assert!(out.contains("src/main.rs"));
    }

    #[test]
    fn trim_history_keeps_system_message() {
        std::env::set_var("AUTOCODING_HISTORY_MAX_MESSAGES", "10");
        std::env::set_var("AUTOCODING_HISTORY_MAX_CHARS", "10000");
        let mut msgs = vec![ChatMessage::system("sys".to_string())];
        for i in 0..30 {
            msgs.push(ChatMessage::user(format!("u{i}")));
        }
        let trimmed = trim_message_history_for_speed(&mut msgs);
        assert!(trimmed.is_some());
        assert_eq!(msgs.first().map(|m| m.role.as_str()), Some("system"));
        std::env::remove_var("AUTOCODING_HISTORY_MAX_MESSAGES");
        std::env::remove_var("AUTOCODING_HISTORY_MAX_CHARS");
    }
}
