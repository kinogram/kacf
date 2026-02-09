use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Ensure that the workspace directory exists. Creates it if necessary.
pub fn ensure_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)
        .with_context(|| format!("create_dir_all {}", dir.display()))?;
    Ok(())
}

/// Write a file within the workspace safely. Prevents directory traversal by
/// rejecting paths containing `..` or absolute prefixes. Creates any
/// necessary parent directories. The file contents are overwritten.
pub fn write_file_safely(workspace: &Path, rel_path: &str, content: &str) -> Result<()> {
    // Reject attempts to escape the workspace via .. or absolute paths.
    if rel_path.contains("..") || rel_path.starts_with('/') || rel_path.starts_with('\\') {
        return Err(anyhow!("Unsafe path: {}", rel_path));
    }
    let full: PathBuf = workspace.join(rel_path);
    // Canonicalize the path to catch sneaky relative components.
    let full = full.canonicalize().unwrap_or(full);
    let ws = workspace.canonicalize().unwrap_or_else(|_| workspace.to_path_buf());
    if !full.starts_with(&ws) {
        return Err(anyhow!("Path escapes workspace: {}", rel_path));
    }
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create parent dirs for {}", parent.display()))?;
    }
    fs::write(&full, content)
        .with_context(|| format!("write {}", full.display()))?;
    Ok(())
}