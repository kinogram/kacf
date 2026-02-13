function setProjectDraftState() {
    const el = document.getElementById('project_dirty_state');
    if (!el) return;
    el.textContent = projectDraftDirty
        ? txt('project_state_dirty', '')
        : txt('project_state_clean', '');
    el.style.color = projectDraftDirty ? '#9a6700' : '';
}

function markProjectDirty() {
    if (projectDraftDirty) return;
    projectDraftDirty = true;
    setProjectDraftState();
}

function markProjectClean() {
    projectDraftDirty = false;
    setProjectDraftState();
}

function shouldProceedWithDirtyDraft(actionText) {
    if (!projectDraftDirty) return true;
    return window.confirm(
        fmt('confirm_dirty_continue', '', { action: actionText })
    );
}

function loadProjects() {
    return Array.isArray(cachedProjects) ? cachedProjects : [];
}

function saveProjects(items) {
    cachedProjects = Array.isArray(items) ? items : [];
}

function ensureWorkspaceForCurrentProject() {
    const sel = document.getElementById('project_selector');
    const id = sel?.value || '';
    if (!id) {
        document.getElementById('workspace').value = '';
        return;
    }
    const item = loadProjects().find(x => x.id === id);
    document.getElementById('workspace').value = item?.workspace || '';
}

function renderProjectAccordion() {
    const container = document.getElementById('project_accordion');
    if (!container) return;
    const all = loadProjects()
        .slice()
        .sort((a, b) => Number(b?.updated_at || 0) - Number(a?.updated_at || 0));
    const key = (projectSearchKeyword || '').trim().toLowerCase();
    const items = key
        ? all.filter(p => {
            const name = (p?.name || '').toLowerCase();
            const ws = (p?.workspace || '').toLowerCase();
            const goal = (p?.goal || '').toLowerCase();
            return name.includes(key) || ws.includes(key) || goal.includes(key);
        })
        : all;
    const selectedId = document.getElementById('project_selector')?.value || '';
    const summary = document.getElementById('project_summary_text');
    if (summary) {
        summary.textContent = key
            ? fmt('project_summary_filtered', '', { shown: items.length, total: all.length })
            : fmt('project_summary_total', '', { total: all.length });
    }
    if (!all.length) {
        container.innerHTML = `<div class="hint">${escapeHtml(txt('project_list_empty', ''))}</div>`;
        return;
    }
    if (!items.length) {
        container.innerHTML = `<div class="hint">${escapeHtml(txt('project_list_no_match', ''))}</div>`;
        return;
    }
    const runningId = (runSessionActive && activeRunLogBucket.startsWith('project:'))
        ? activeRunLogBucket.slice('project:'.length)
        : '';
    container.innerHTML = items
        .map(p => {
            const active = p.id === selectedId;
            const running = runningId && p.id === runningId;
            const dt = p.updated_at ? new Date(p.updated_at * 1000).toLocaleString() : '-';
            const snap = p.snapshot || {};
            const leaf = workspaceLeaf(p.workspace || '-');
            const goalText = (p.goal || '').trim() || '-';
            return `
<div class="project-card ${active ? 'active' : ''} ${running ? 'running' : ''}">
  <div class="project-head">
    <div class="project-title" onclick="selectProject('${p.id}', true)">${escapeHtml(p.name || txt('project_unnamed', ''))} <span style="color:#5c6f8f">[${escapeHtml(leaf)}]</span></div>
    <div class="project-tags">
      ${active ? `<span class="tag active">${escapeHtml(txt('tag_current', ''))}</span>` : ''}
      ${running ? `<span class="tag running">${escapeHtml(txt('tag_running', ''))}</span>` : ''}
    </div>
  </div>
  <div class="project-goal" title="${escapeHtml(goalText)}">${escapeHtml(txt('project_goal_prefix', ''))}${escapeHtml(goalText)}</div>
  <div class="meta">
    ${escapeHtml(txt('project_meta_workspace', ''))}: ${escapeHtml(p.workspace || '-')}<br>
    ${escapeHtml(txt('project_meta_updated', ''))}: ${escapeHtml(dt)}<br>
    ${escapeHtml(txt('project_meta_eval', ''))}: ${escapeHtml(snap.eval_cmd || '-')}<br>
    ${escapeHtml(txt('project_meta_revert', ''))}: ${escapeHtml(snap.auto_revert_profile || '-')}
  </div>
  <div class="buttons">
    <button class="btn-ghost" onclick="selectProject('${p.id}', true)">${escapeHtml(txt('btn_project_open_card', ''))}</button>
    <button class="btn-ghost" onclick="deleteProjectById('${p.id}')">${escapeHtml(txt('btn_project_delete_card', ''))}</button>
  </div>
</div>`;
        })
        .join('');
}

function selectProject(id, autoLoad) {
    const sel = document.getElementById('project_selector');
    if (!sel) return;
    if (autoLoad && !shouldProceedWithDirtyDraft(txt('action_open_project', ''))) return;
    sel.value = id;
    syncProjectNameFromSelection();
    if (autoLoad) loadSelectedProject(true);
}

function deleteProjectById(id) {
    const sel = document.getElementById('project_selector');
    if (!sel) return;
    sel.value = id;
    deleteSelectedProject();
}

async function fetchProjectsFromServer() {
    try {
        const resp = await fetch('/projects');
        if (!resp.ok) return null;
        const data = await resp.json();
        if (!Array.isArray(data)) return null;
        return data;
    } catch (_e) {
        return null;
    }
}

async function refreshProjectsFromServer() {
    const data = await fetchProjectsFromServer();
    if (!data) return false;
    saveProjects(data);
    return true;
}

async function upsertProjectToServer(project) {
    try {
        const resp = await fetch('/projects', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(project),
        });
        return resp.ok;
    } catch (_e) {
        return false;
    }
}

async function deleteProjectOnServer(id) {
    try {
        const resp = await fetch(`/projects/${encodeURIComponent(id)}`, { method: 'DELETE' });
        return resp.ok;
    } catch (_e) {
        return false;
    }
}

function randomProjectId() {
    return `p_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
}

function renderProjectSelector(selectedId) {
    const sel = document.getElementById('project_selector');
    if (!sel) return;
    const items = loadProjects();
    sel.innerHTML = '';
    const empty = document.createElement('option');
    empty.value = '';
    empty.textContent = items.length ? txt('project_selector_placeholder', '') : txt('project_selector_empty', '');
    sel.appendChild(empty);
    items.forEach(p => {
        const opt = document.createElement('option');
        opt.value = p.id;
        const ws = (p.workspace || '').trim();
        const name = p.name || txt('project_unnamed', '');
        opt.textContent = ws ? `${name} (${ws})` : name;
        sel.appendChild(opt);
    });
    if (selectedId && items.some(x => x.id === selectedId)) {
        sel.value = selectedId;
    } else {
        sel.value = '';
    }
    ensureWorkspaceForCurrentProject();
    renderProjectAccordion();
}

function syncProjectNameFromSelection() {
    const sel = document.getElementById('project_selector');
    const items = loadProjects();
    const found = items.find(x => x.id === sel.value);
    if (found) {
        document.getElementById('project_name').value = found.name || '';
        projectNameManualOverride = true;
        lastAutoProjectName = '';
    } else {
        document.getElementById('project_name').value = '';
        projectNameManualOverride = false;
        lastAutoProjectName = '';
        autoFillProjectNameFromGoal(false);
    }
    renderCurrentLogView();
    renderUiForViewBucket();
    applyReadOnlyMode();
}

async function saveCurrentProject(opts) {
    const options = opts || {};
    const silent = !!options.silent;
    updateSharedConfigFromInputs();
    autoFillProjectNameFromGoal(false);
    const body = { ...getFormData() };
    const sel = document.getElementById('project_selector');
    const items = loadProjects();
    const selectedId = sel.value;
    const idx = items.findIndex(x => x.id === selectedId);
    const inputName = document.getElementById('project_name').value.trim();
    const fallbackName = inputName || fmt('project_default_name', '', { index: items.length + 1 });
    const projectId = idx >= 0 ? items[idx].id : randomProjectId();
    let projectWorkspace = '';
    if (idx >= 0 && items[idx].workspace) {
        projectWorkspace = items[idx].workspace;
    } else {
        const slug = await suggestProjectSlug(inputName || fallbackName, body.goal || '');
        const basePath = `${WORKSPACE_ROOT}/${slug}`;
        projectWorkspace = ensureUniqueWorkspace(basePath, items, projectId);
    }
    body.workspace = projectWorkspace;
    const snapshot = {
        ...body,
        api_key: '',
        base_url: '',
        model: '',
        workspace: projectWorkspace,
    };
    const project = {
        id: projectId,
        name: inputName || fallbackName,
        workspace: projectWorkspace,
        goal: body.goal,
        updated_at: Date.now(),
        snapshot,
    };
    if (idx >= 0) {
        items[idx] = project;
    } else {
        items.unshift(project);
    }
    saveProjects(items);
    const serverOk = await upsertProjectToServer(project);
    if (serverOk) {
        await refreshProjectsFromServer();
    }
    renderProjectSelector(project.id);
    document.getElementById('project_name').value = project.name;
    projectNameManualOverride = true;
    lastAutoProjectName = '';
    document.getElementById('workspace').value = projectWorkspace;
    if (!silent) {
        appendLog(
            txt('log_project_saved', '')
                .replace('{name}', project.name)
                .replace('{suffix}', serverOk ? '' : txt('log_fallback_suffix_unsynced', ''))
        );
    }
    markProjectClean();
    closeSidebarOnNarrow();
    return true;
}

function loadSelectedProject(skipDirtyCheck) {
    const bypass = skipDirtyCheck === true;
    if (!bypass && !shouldProceedWithDirtyDraft(txt('action_load_project', ''))) return;
    updateSharedConfigFromInputs();
    const sel = document.getElementById('project_selector');
    const items = loadProjects();
    const found = items.find(x => x.id === sel.value);
    if (!found || !found.snapshot) {
        setStatus(txt('status_need_select_project', ''), 'status-danger');
        return;
    }
    const merged = {
        ...found.snapshot,
        api_key: sharedConfig.api_key,
        base_url: sharedConfig.base_url,
        model: sharedConfig.model,
        workspace: found.workspace || '',
    };
    applyFormData(merged);
    document.getElementById('project_name').value = found.name || '';
    projectNameManualOverride = true;
    lastAutoProjectName = '';
    ensureWorkspaceForCurrentProject();
    scheduleDraftSave();
    markProjectClean();
    setStatus(fmt('status_project_loaded', '', { name: found.name || txt('project_unnamed', '') }), 'status-warn');
    appendLog(txt('log_project_loaded', '').replace('{name}', (found.name || txt('project_unnamed', ''))));
    renderCurrentLogView();
    renderUiForViewBucket();
    applyReadOnlyMode();
    closeSidebarOnNarrow();
}

async function deleteSelectedProject() {
    if (!shouldProceedWithDirtyDraft(txt('action_delete_project', ''))) return;
    const sel = document.getElementById('project_selector');
    const items = loadProjects();
    const found = items.find(x => x.id === sel.value);
    if (!found) {
        setStatus(txt('status_need_delete_project', ''), 'status-danger');
        return;
    }
    const confirmed = window.confirm(
        fmt(
            'confirm_delete_project',
            txt('confirm_delete_project', ''),
            { name: found.name || txt('project_unnamed', '') }
        )
    );
    if (!confirmed) return;
    const next = items.filter(x => x.id !== found.id);
    saveProjects(next);
    const serverOk = await deleteProjectOnServer(found.id);
    if (serverOk) {
        await refreshProjectsFromServer();
    }
    renderProjectSelector('');
    document.getElementById('project_name').value = '';
    projectNameManualOverride = false;
    lastAutoProjectName = '';
    markProjectClean();
    const logs = loadProjectLogs();
    delete logs[`project:${found.id}`];
    delete logBucketTouchedAt[`project:${found.id}`];
    delete logRenderStateByBucket[`project:${found.id}`];
    saveProjectLogs(logs);
    const uiMap = loadProjectUiStateMap();
    delete uiMap[`project:${found.id}`];
    delete uiStateBucketTouchedAt[`project:${found.id}`];
    saveProjectUiStateMap(uiMap);
    renderCurrentLogView();
    renderUiForViewBucket();
    appendLog(
        txt('log_project_deleted', '')
            .replace('{name}', (found.name || txt('project_unnamed', '')))
            .replace('{suffix}', serverOk ? '' : txt('log_fallback_suffix_unsynced', ''))
    );
    closeSidebarOnNarrow();
}

function createNewProject() {
    if (!shouldProceedWithDirtyDraft(txt('action_new_project', ''))) return;
    const cur = getFormData();
    const defaults = defaultFormData();
    defaults.api_key = cur.api_key;
    defaults.base_url = cur.base_url;
    defaults.model = cur.model;
    applyFormData(defaults);
    document.getElementById('project_selector').value = '';
    document.getElementById('project_name').value = '';
    projectNameManualOverride = false;
    lastAutoProjectName = '';
    document.getElementById('workspace').value = '';
    activeRunLogBucket = '';
    scheduleDraftSave();
    autoFillProjectNameFromGoal(false);
    renderCurrentLogView();
    renderUiForViewBucket();
    applyReadOnlyMode();
    markProjectClean();
    closeSidebarOnNarrow();
    setStatus(txt('status_new_project', ''), 'status-warn');
}

function shouldAutoSaveProjectNow() {
    if (isReadOnlyView() || runSessionActive) return false;
    const selectedId = document.getElementById('project_selector')?.value || '';
    if (selectedId) return true;
    const goal = (document.getElementById('goal')?.value || '').trim();
    const name = (document.getElementById('project_name')?.value || '').trim();
    return !!(goal || name);
}

async function autoSaveProjectFromForm() {
    if (!shouldAutoSaveProjectNow()) return;
    if (projectAutoSaveInFlight) return;
    projectAutoSaveInFlight = true;
    try {
        const ok = await saveCurrentProject({ silent: true });
        if (ok) {
            setAutoSaveState(
                fmt('autosave_project_ok', '', { time: nowText() })
            );
        } else {
            setAutoSaveState(
                fmt('autosave_project_fail', '', { time: nowText() })
            );
        }
    } catch (_e) {
        setAutoSaveState(
            fmt('autosave_project_fail', '', { time: nowText() })
        );
    } finally {
        projectAutoSaveInFlight = false;
    }
}

function scheduleDraftSave() {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
        setAutoSaveState(fmt('autosave_project_pending', '', { time: nowText() }));
    }, 400);
    if (projectAutoSaveTimer) clearTimeout(projectAutoSaveTimer);
    projectAutoSaveTimer = setTimeout(() => {
        autoSaveProjectFromForm();
    }, 550);
}

async function loadProjectConfigForWorkspace() {
    const ws = document.getElementById('workspace').value.trim();
    if (!ws) return;
    try {
        const resp = await fetch(`/project_config?workspace=${encodeURIComponent(ws)}`);
        if (!resp.ok) return;
        const cfg = await resp.json();
        if (cfg && typeof cfg.auto_revert_profile === 'string' && cfg.auto_revert_profile) {
            document.getElementById('auto_revert_profile').value = cfg.auto_revert_profile;
        }
        if (cfg && typeof cfg.precheck_cmd === 'string') {
            document.getElementById('precheck_cmd').value = cfg.precheck_cmd;
        }
        if (cfg && typeof cfg.release_gate_threshold === 'string') {
            document.getElementById('release_gate_threshold').value = cfg.release_gate_threshold;
        }
        if (cfg && typeof cfg.unattended_mode === 'boolean') {
            document.getElementById('unattended_mode').checked = cfg.unattended_mode;
        }
    } catch (_e) {}
}

window.KACF = window.KACF || {};
window.KACF.projects = {
    setProjectDraftState,
    markProjectDirty,
    markProjectClean,
    shouldProceedWithDirtyDraft,
    loadProjects,
    saveProjects,
    ensureWorkspaceForCurrentProject,
    renderProjectAccordion,
    selectProject,
    deleteProjectById,
    fetchProjectsFromServer,
    refreshProjectsFromServer,
    upsertProjectToServer,
    deleteProjectOnServer,
    randomProjectId,
    renderProjectSelector,
    syncProjectNameFromSelection,
    saveCurrentProject,
    loadSelectedProject,
    deleteSelectedProject,
    createNewProject,
    shouldAutoSaveProjectNow,
    autoSaveProjectFromForm,
    scheduleDraftSave,
    loadProjectConfigForWorkspace,
};
