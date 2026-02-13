use anyhow::{anyhow, Result};
use std::collections::BTreeSet;

pub(crate) fn validate_patch_payload(summary: &str, diff: &str) -> Result<()> {
    if summary.trim().is_empty() {
        return Err(anyhow!("summary 不能为空"));
    }
    if diff.trim().is_empty() {
        return Err(anyhow!("diff 不能为空"));
    }
    if diff.len() > 1_500_000 {
        return Err(anyhow!("diff 体积过大: {} bytes", diff.len()));
    }
    if !diff.contains("diff --git ") {
        return Err(anyhow!("diff 缺少 `diff --git` 头"));
    }
    if !diff.contains("\n@@ ") && !diff.contains("\n@@\t") && !diff.contains("\n@@\n") && !diff.contains("\n@@") {
        return Err(anyhow!("diff 缺少 hunk（@@）"));
    }
    let paths = extract_paths_from_unified_diff(diff)?;
    if paths.is_empty() {
        return Err(anyhow!("diff 未识别到任何文件路径"));
    }
    if paths.len() > 120 {
        return Err(anyhow!("单次 patch 文件数量过多: {}", paths.len()));
    }
    Ok(())
}

pub(crate) fn extract_paths_from_unified_diff(diff: &str) -> Result<Vec<String>> {
    let mut out = BTreeSet::new();
    for line in diff.lines() {
        if !line.starts_with("diff --git ") {
            continue;
        }
        let rest = line.trim_start_matches("diff --git ").trim();
        let mut parts = rest.split_whitespace();
        let a = parts.next().unwrap_or("");
        let b = parts.next().unwrap_or("");
        if a.is_empty() || b.is_empty() {
            return Err(anyhow!("非法 diff 头: {}", line));
        }
        let raw = if b == "/dev/null" { a } else { b };
        let path = raw
            .strip_prefix("a/")
            .or_else(|| raw.strip_prefix("b/"))
            .unwrap_or(raw)
            .trim();
        if path.is_empty() || path.starts_with('/') || path.contains("..") {
            return Err(anyhow!("非法路径: {}", path));
        }
        out.insert(path.to_string());
    }
    Ok(out.into_iter().collect())
}

pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let head = &s[..end];
    let truncated_chars = s[end..].chars().count();
    format!("{}...\n[truncated {} chars]", head, truncated_chars)
}

pub(crate) fn extract_json(input: &str) -> Result<String> {
    if serde_json::from_str::<serde_json::Value>(input).is_ok() {
        return Ok(input.to_string());
    }
    if let (Some(start), Some(end)) = (input.find('{'), input.rfind('}')) {
        if start < end {
            let candidate = &input[start..=end];
            if serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
                return Ok(candidate.to_string());
            }
        }
    }
    Err(anyhow!("未找到合法 JSON 段"))
}

#[cfg(test)]
mod tests {
    use super::{extract_paths_from_unified_diff, truncate, validate_patch_payload};

    #[test]
    fn truncate_handles_utf8_boundary_safely() {
        let s = "abc数字xyz";
        let out = truncate(s, 4);
        assert!(out.starts_with("abc..."));
        assert!(out.contains("[truncated"));
    }

    #[test]
    fn truncate_keeps_short_strings_unchanged() {
        let s = "hello";
        assert_eq!(truncate(s, 5), "hello");
        assert_eq!(truncate(s, 8), "hello");
    }

    #[test]
    fn validate_patch_payload_accepts_unified_diff() {
        let diff = "\
diff --git a/src/main.rs b/src/main.rs\n\
index 1111111..2222222 100644\n\
--- a/src/main.rs\n\
+++ b/src/main.rs\n\
@@ -1 +1 @@\n\
-fn main() {}\n\
+fn main(){ println!(\"ok\"); }\n";
        assert!(validate_patch_payload("fix", diff).is_ok());
        let paths = extract_paths_from_unified_diff(diff).expect("paths");
        assert_eq!(paths, vec!["src/main.rs".to_string()]);
    }
}
