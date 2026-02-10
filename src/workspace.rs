use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Ensure that the workspace directory exists. Creates it if necessary.
pub fn ensure_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("create_dir_all {}", dir.display()))?;
    Ok(())
}

/// Write a file within the workspace safely. Prevents directory traversal by
/// rejecting paths containing `..` or absolute prefixes. Creates any
/// necessary parent directories. The file contents are overwritten.
pub fn write_file_safely(workspace: &Path, rel_path: &str, content: &str) -> Result<()> {
    if rel_path.trim().is_empty() {
        return Err(anyhow!("Empty path"));
    }
    // Ensure workspace exists and use canonical root for safety checks.
    fs::create_dir_all(workspace)
        .with_context(|| format!("create workspace {}", workspace.display()))?;
    let ws = workspace
        .canonicalize()
        .with_context(|| format!("canonicalize workspace {}", workspace.display()))?;

    // Reject absolute/prefix/parent traversal components.
    let rel = Path::new(rel_path);
    if rel.is_absolute() {
        return Err(anyhow!("Unsafe absolute path: {}", rel_path));
    }
    for c in rel.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err(anyhow!("Unsafe path component in: {}", rel_path)),
        }
    }

    // Block symlink traversal in any existing ancestor path segment.
    let mut check = ws.clone();
    let mut components = rel.components().peekable();
    while let Some(component) = components.next() {
        if let Component::Normal(part) = component {
            check.push(part);
            if components.peek().is_some()
                && fs::symlink_metadata(&check)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false)
            {
                return Err(anyhow!("Symlink traversal is not allowed: {}", rel_path));
            }
        }
    }

    let full: PathBuf = ws.join(rel);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create parent dirs for {}", parent.display()))?;
        let parent_canon = parent
            .canonicalize()
            .with_context(|| format!("canonicalize parent {}", parent.display()))?;
        if !parent_canon.starts_with(&ws) {
            return Err(anyhow!("Path escapes workspace: {}", rel_path));
        }
    } else {
        return Err(anyhow!("Invalid path: {}", rel_path));
    }

    if fs::symlink_metadata(&full)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(anyhow!("Refusing to write through symlink: {}", rel_path));
    }
    fs::write(&full, content).with_context(|| format!("write {}", full.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(prefix: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        p.push(format!(
            "autocoding_{}_{}_{}",
            prefix,
            std::process::id(),
            nanos
        ));
        p
    }

    #[test]
    fn allows_normal_write_inside_workspace() {
        let root = temp_dir("ws_ok");
        let ws = root.join("workspace");
        fs::create_dir_all(&ws).expect("create workspace");
        write_file_safely(&ws, "nested/file.txt", "hello").expect("write succeeds");
        let content = fs::read_to_string(ws.join("nested/file.txt")).expect("read file");
        assert_eq!(content, "hello");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_parent_traversal() {
        let root = temp_dir("ws_parent");
        let ws = root.join("workspace");
        fs::create_dir_all(&ws).expect("create workspace");
        let err = write_file_safely(&ws, "../escape.txt", "nope").expect_err("must fail");
        assert!(err.to_string().contains("Unsafe"));
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = temp_dir("ws_symlink");
        let ws = root.join("workspace");
        let outside = root.join("outside");
        fs::create_dir_all(&ws).expect("create workspace");
        fs::create_dir_all(&outside).expect("create outside");
        symlink(&outside, ws.join("link")).expect("create symlink");

        let err = write_file_safely(&ws, "link/pwn.txt", "owned").expect_err("must fail");
        assert!(err.to_string().contains("Symlink traversal"));
        assert!(!outside.join("pwn.txt").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
