use std::fs;
use std::path::{Path, PathBuf};

use crate::workspace;

pub(crate) fn normalize_rel_path(path: &Path) -> Option<PathBuf> {
    if path.is_absolute() {
        return None;
    }
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(seg) => out.push(seg),
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() {
        return None;
    }
    Some(out)
}

pub(crate) fn workspace_path_for_input(workspace: &str, managed_root_dir: &str) -> Option<PathBuf> {
    let ws = workspace.trim();
    if ws.is_empty() {
        return None;
    }
    let rel = normalize_rel_path(Path::new(ws))?;
    let root_rel = normalize_rel_path(Path::new(managed_root_dir))?;
    if !rel.starts_with(&root_rel) {
        return None;
    }
    Some(rel)
}

pub(crate) fn require_managed_workspace(
    workspace: &str,
    managed_root_dir: &str,
    managed_workspaces_dir: &str,
) -> Result<PathBuf, String> {
    let rel = workspace_path_for_input(workspace, managed_root_dir).ok_or_else(|| {
        format!(
            "workspace must be under ./{}/{}",
            managed_root_dir, managed_workspaces_dir
        )
    })?;
    Ok(std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(rel))
}

pub(crate) fn save_project_config(
    workspace: &str,
    managed_root_dir: &str,
    managed_workspaces_dir: &str,
    project_config_filename: &str,
    cfg: &crate::web_ui::ProjectConfig,
) -> std::io::Result<()> {
    let Ok(ws) = require_managed_workspace(workspace, managed_root_dir, managed_workspaces_dir)
    else {
        return Ok(());
    };
    fs::create_dir_all(&ws)?;
    let json = serde_json::to_string_pretty(cfg)?;
    let path = ws.join(project_config_filename);
    workspace::atomic_write_text(&path, &json)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(())
}

pub(crate) fn read_project_config(
    workspace: &str,
    managed_root_dir: &str,
    managed_workspaces_dir: &str,
    project_config_filename: &str,
) -> Option<crate::web_ui::ProjectConfig> {
    let ws = require_managed_workspace(workspace, managed_root_dir, managed_workspaces_dir).ok()?;
    let path = ws.join(project_config_filename);
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str::<crate::web_ui::ProjectConfig>(&content).ok()
}

pub(crate) fn read_ui_cache(
    managed_root_dir: &str,
    ui_cache_filename: &str,
) -> crate::web_ui::UiCachePayload {
    let path = ui_cache_path(managed_root_dir, ui_cache_filename);
    let content = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return crate::web_ui::UiCachePayload::default(),
    };
    serde_json::from_str::<crate::web_ui::UiCachePayload>(&content).unwrap_or_default()
}

pub(crate) fn write_ui_cache(
    managed_root_dir: &str,
    ui_cache_filename: &str,
    payload: &crate::web_ui::UiCachePayload,
) -> std::io::Result<()> {
    let path = ui_cache_path(managed_root_dir, ui_cache_filename);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(payload)?;
    workspace::atomic_write_text(&path, &json)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(())
}

fn ui_cache_path(managed_root_dir: &str, ui_cache_filename: &str) -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(managed_root_dir)
        .join(ui_cache_filename)
}

#[cfg(test)]
mod tests {
    use super::{normalize_rel_path, require_managed_workspace, workspace_path_for_input};
    use std::path::Path;

    #[test]
    fn normalize_rel_path_rejects_escape_components() {
        assert!(normalize_rel_path(Path::new("../x")).is_none());
        assert!(normalize_rel_path(Path::new("/abs/x")).is_none());
        assert!(normalize_rel_path(Path::new("a/../../b")).is_none());
    }

    #[test]
    fn workspace_path_must_stay_under_managed_root() {
        assert!(
            workspace_path_for_input("autocoding_data/workspaces/a", "autocoding_data").is_some()
        );
        assert!(workspace_path_for_input("workspace/a", "autocoding_data").is_none());
    }

    #[test]
    fn require_managed_workspace_returns_helpful_error() {
        let err = require_managed_workspace("workspace/a", "autocoding_data", "workspaces")
            .expect_err("must reject unmanaged workspace");
        assert!(err.contains("./autocoding_data/workspaces"));
    }
}
