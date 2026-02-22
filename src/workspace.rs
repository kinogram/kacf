use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Ensure that the workspace directory exists. Creates it if necessary.
pub fn ensure_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("create_dir_all {}", dir.display()))?;
    Ok(())
}

/// Atomically writes a file by writing to a same-directory temp file and renaming.
/// Temp files use a unique suffix to avoid concurrent writers clobbering each other.
pub fn atomic_write_text(path: &Path, content: &str) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("create parent dirs for {}", parent.display()))?;
    let tmp = atomic_tmp_path(path);
    fs::write(&tmp, content).with_context(|| format!("write temp {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| {
        format!(
            "rename temp {} -> {}",
            tmp.display(),
            path.display()
        )
    })?;
    Ok(())
}

fn atomic_tmp_path(path: &Path) -> PathBuf {
    let stem = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("kacf");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    path.with_file_name(format!("{stem}.tmp.{}.{}", std::process::id(), nanos))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn atomic_write_creates_parent_and_writes_content() {
        let root = temp_dir("ws_ok");
        let target = root.join("workspace/nested/file.txt");
        atomic_write_text(&target, "hello").expect("write succeeds");
        let content = fs::read_to_string(target).expect("read file");
        assert_eq!(content, "hello");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn atomic_write_overwrites_existing_content() {
        let root = temp_dir("ws_overwrite");
        let target = root.join("workspace/file.txt");
        atomic_write_text(&target, "v1").expect("first write");
        atomic_write_text(&target, "v2").expect("second write");
        let content = fs::read_to_string(target).expect("read file");
        assert_eq!(content, "v2");
        let _ = fs::remove_dir_all(&root);
    }
}
