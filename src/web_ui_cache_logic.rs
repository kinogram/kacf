pub(crate) fn prepare_ui_cache_for_response(_payload: &mut crate::web_ui::UiCachePayload) {}

pub(crate) fn apply_ui_cache_patch(
    payload: &mut crate::web_ui::UiCachePayload,
    patch: crate::web_ui::UiCachePatch,
) {
    if let Some(v) = patch.projects {
        payload.projects = v;
    }
    if let Some(v) = patch.shared_config {
        payload.shared_config = Some(v);
    }
    if let Some(v) = patch.project_logs {
        payload.project_logs = v;
    }
    if let Some(v) = patch.project_ui_state {
        payload.project_ui_state = v;
    }
}
