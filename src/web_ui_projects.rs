pub(crate) fn sort_projects_by_updated_desc(items: &mut [crate::web_ui::WebProject]) {
    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
}

pub(crate) fn normalize_project_for_upsert(
    mut item: crate::web_ui::WebProject,
    now_unix: u64,
) -> Result<crate::web_ui::WebProject, &'static str> {
    if item.id.trim().is_empty() {
        return Err("project id is empty");
    }
    if item.name.trim().is_empty() {
        item.name = item.workspace.clone();
    }
    item.updated_at = now_unix;
    Ok(item)
}

pub(crate) fn upsert_project_in_cache(
    cache: &mut crate::web_ui::UiCachePayload,
    item: crate::web_ui::WebProject,
) {
    if let Some(idx) = cache.projects.iter().position(|p| p.id == item.id) {
        cache.projects[idx] = item;
    } else {
        cache.projects.push(item);
    }
}

pub(crate) fn delete_project_from_cache(
    cache: &mut crate::web_ui::UiCachePayload,
    id: &str,
) -> Result<(), &'static str> {
    if id.trim().is_empty() {
        return Err("project id is empty");
    }
    let before = cache.projects.len();
    cache.projects.retain(|p| p.id != id);
    if cache.projects.len() == before {
        return Err("project not found");
    }
    Ok(())
}
