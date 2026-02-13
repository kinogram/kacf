use std::path::Path;

pub(crate) fn prepare_ui_cache_for_response(
    payload: &mut crate::web_ui::UiCachePayload,
    managed_root: &Path,
) {
    if let Some(cfg) = payload.shared_config.as_mut() {
        crate::web_ui_api_key::decrypt_shared_config_for_response(cfg, managed_root);
        if cfg.mask_api_key && !cfg.api_key.trim().is_empty() {
            cfg.api_key = crate::web_ui_api_key::mask_api_key_for_display(&cfg.api_key);
            cfg.api_key_is_masked = true;
        }
    }
}

pub(crate) fn plain_api_key_from_cache(
    payload: &mut crate::web_ui::UiCachePayload,
    managed_root: &Path,
) -> String {
    if let Some(cfg) = payload.shared_config.as_mut() {
        crate::web_ui_api_key::decrypt_shared_config_for_response(cfg, managed_root);
        return cfg.api_key.clone();
    }
    String::new()
}

pub(crate) fn apply_ui_cache_patch(
    payload: &mut crate::web_ui::UiCachePayload,
    patch: crate::web_ui::UiCachePatch,
    managed_root: &Path,
) {
    if let Some(v) = patch.projects {
        payload.projects = v;
    }
    if let Some(v) = patch.shared_config {
        let mut cfg = v;
        if cfg.api_key_is_masked {
            if let Some(prev) = payload.shared_config.as_ref() {
                cfg.api_key = prev.api_key.clone();
            } else {
                cfg.api_key.clear();
            }
        }
        cfg.api_key_is_masked = false;
        crate::web_ui_api_key::encrypt_shared_config_for_storage(&mut cfg, managed_root);
        payload.shared_config = Some(cfg);
    }
    if let Some(v) = patch.project_logs {
        payload.project_logs = v;
    }
    if let Some(v) = patch.project_ui_state {
        payload.project_ui_state = v;
    }
}
