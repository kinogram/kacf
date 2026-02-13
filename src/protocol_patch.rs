use anyhow::{anyhow, Result};
use std::collections::HashSet;

use crate::protocol::FileWrite;

pub(crate) fn validate_patch_payload(summary: &str, files: &[FileWrite]) -> Result<()> {
    if summary.trim().is_empty() {
        return Err(anyhow!("summary 不能为空"));
    }
    if files.is_empty() {
        return Err(anyhow!("files 不能为空"));
    }
    if files.len() > 80 {
        return Err(anyhow!("单次 patch 文件数量过多: {}", files.len()));
    }
    let mut seen = HashSet::new();
    let mut total_bytes = 0usize;
    for f in files {
        let path = f.path.trim();
        if path.is_empty() {
            return Err(anyhow!("存在空 path"));
        }
        if !seen.insert(path.to_string()) {
            return Err(anyhow!("path 重复: {}", path));
        }
        if f.content.len() > 700_000 {
            return Err(anyhow!("单文件过大: {} ({} bytes)", path, f.content.len()));
        }
        total_bytes += f.content.len();
    }
    if total_bytes > 3_000_000 {
        return Err(anyhow!("本次 patch 总体积过大: {} bytes", total_bytes));
    }
    Ok(())
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
    use super::truncate;

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
}
