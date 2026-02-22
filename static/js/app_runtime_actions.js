(() => {
function assertRuntimeActionDependencies() {
    const root = window.KACF || {};
    const required = {
        state: [
            'txt',
            'fmt',
            'isReadOnlyView',
            'goToRunningProjectView',
            'currentLogBucket',
            'viewLogBucket',
            'setRunActionButtons',
            'setRunningProjectIndicator',
            'setProjectControlsDisabled',
            'applyReadOnlyMode',
            'updateGoRunningProjectButton',
            'autoFillProjectNameFromGoal',
            'applySharedConfigToInputs',
            'initLanguagePack',
            'isNarrowViewport',
            'setSidebarCollapsed',
            'setSidebarMobileOpen',
            'applySidebarLayout',
            'normalizeLanguageCode',
            'switchLanguage',
            'renderLanguageOptions',
            'updateSharedConfigFromInputs',
            'maybeWarnLargeCharLimit',
        ],
        uiCache: [
            'fetchUiCacheFromServer',
            'writeBucketUiState',
            'renderClarifyQuestions',
            'renderUiForViewBucket',
            'resetBucketRuntimeUiState',
            'bindGlobalOptionInput',
            'applyGlobalOptionsToInputs',
            'scheduleUiCacheSave',
        ],
        logPipeline: ['setStatus', 'setRunState', 'appendLog', 'renderCurrentLogView'],
	        runtimeState: [
	            'stopAfterMinutesSetting',
	            'markRunSessionStarted',
	            'clearUnattendedAutoResumeTimer',
	            'autoResumeAttemptsSetting',
	            'armManualRunStopTimer',
	            'renderUnattendedState',
	            'resetRunSessionUiState',
	            'resetStopTimerState',
	            'resetUnattendedRunState',
	            'scheduleUnattendedAutoResume',
	            'applyInterruptedTerminalState',
	            'setAutoSaveState',
	            'openGlobalConfigModal',
	            'saveGlobalConfig',
	            'closeGlobalConfigModal',
	            'getFormData',
	        ],
        projects: [
            'ensureWorkspaceForCurrentProject',
            'saveCurrentProject',
            'refreshProjectsFromServer',
            'renderProjectSelector',
            'loadSelectedProject',
            'deleteSelectedProject',
            'createNewProject',
            'syncProjectNameFromSelection',
            'loadProjects',
            'loadProjectConfigForWorkspace',
            'setProjectDraftState',
            'markProjectClean',
            'markProjectDirty',
            'scheduleDraftSave',
        ],
        runtimeSync: [
            'syncSharedConfigForRun',
            'prepareFormDataWithWorkspace',
            'primeRunRequestContext',
            'clearStopAckTimer',
            'refreshUiState',
            'refreshMetrics',
            'bootstrapEventCursor',
            'monitorRealtimeChannel',
            'startEventStream',
            'closeEventStream',
        ],
        vm: [
            'init',
        ],
    };
    Object.entries(required).forEach(([scope, methods]) => {
        const api = root[scope];
        if (!api || typeof api !== 'object') {
            throw new Error(`KACF.${scope} namespace missing`);
        }
        methods.forEach((name) => {
            if (typeof api[name] !== 'function') {
                throw new Error(`KACF.${scope}.${name} is not available`);
            }
        });
    });
}

assertRuntimeActionDependencies();

const {
    txt,
    fmt,
    isReadOnlyView,
    goToRunningProjectView,
    currentLogBucket,
    viewLogBucket,
    setRunActionButtons,
    setRunningProjectIndicator,
    setProjectControlsDisabled,
    applyReadOnlyMode,
    updateGoRunningProjectButton,
    autoFillProjectNameFromGoal,
    applySharedConfigToInputs,
    initLanguagePack,
    isNarrowViewport,
    setSidebarCollapsed,
    setSidebarMobileOpen,
    applySidebarLayout,
    normalizeLanguageCode,
    switchLanguage,
    renderLanguageOptions,
    updateSharedConfigFromInputs,
    maybeWarnLargeCharLimit,
} = window.KACF.state;
const {
    fetchUiCacheFromServer,
    writeBucketUiState,
    renderClarifyQuestions,
    renderUiForViewBucket,
    resetBucketRuntimeUiState,
    bindGlobalOptionInput,
    applyGlobalOptionsToInputs,
    scheduleUiCacheSave,
} = window.KACF.uiCache;
const { setStatus, setRunState, appendLog, renderCurrentLogView } = window.KACF.logPipeline;
const {
    stopAfterMinutesSetting,
    markRunSessionStarted,
    clearUnattendedAutoResumeTimer,
    autoResumeAttemptsSetting,
    armManualRunStopTimer,
    renderUnattendedState,
    resetRunSessionUiState,
    resetStopTimerState,
	    scheduleUnattendedAutoResume,
	    applyInterruptedTerminalState,
	    resetUnattendedRunState,
	    setAutoSaveState,
	    openGlobalConfigModal,
	    saveGlobalConfig,
	    closeGlobalConfigModal,
    getFormData,
} = window.KACF.runtimeState;
const { init: initVmPanel } = window.KACF.vm;
const {
    ensureWorkspaceForCurrentProject,
    saveCurrentProject,
    refreshProjectsFromServer,
    renderProjectSelector,
    loadSelectedProject,
    deleteSelectedProject,
    createNewProject,
    syncProjectNameFromSelection,
    loadProjects,
    loadProjectConfigForWorkspace,
    setProjectDraftState,
    markProjectClean,
    markProjectDirty,
    scheduleDraftSave,
} = window.KACF.projects;

function runtimeSyncApi() {
    return (window.KACF && window.KACF.runtimeSync) || {};
}

function runtimeSyncCall(name, ...args) {
    const api = runtimeSyncApi();
    if (typeof api[name] !== 'function') {
        throw new Error(`KACF.runtimeSync.${name} is not available`);
    }
    return api[name](...args);
}

async function startSession() {
    ensureWorkspaceForCurrentProject();
    if (isReadOnlyView()) {
        setStatus(txt('status_readonly_view', ''), 'status-danger');
        return;
    }
    if (!(await runtimeSyncCall('syncSharedConfigForRun', 'status_start_shared_config_sync_failed'))) {
        return;
    }
    const body = await runtimeSyncCall('prepareFormDataWithWorkspace', 'status_start_alloc_workspace_failed');
    if (!body) {
        return;
    }
    if (!body.api_key) {
        setStatus(txt('status_start_api_key_empty', ''), 'status-danger');
        return;
    }
    if (stopAfterMinutesSetting() === 0) {
        const proceed = window.confirm(txt('warn_stop_timer_disabled', ''));
        if (!proceed) return;
    }
    try {
        runtimeSyncCall('primeRunRequestContext');
        const resp = await fetch('/start', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body),
        });
        if (!resp.ok) {
            const msg = await resp.text();
            throw new Error(`${resp.status} ${msg}`);
        }
        setStatus(txt('status_started_new_task', ''), 'status-warn');
        markRunSessionStarted(activeRunProjectLabel, true);
        activeRunUnattendedMode = !!body.unattended_mode;
        unattendedAutoResumeRemaining = autoResumeAttemptsSetting();
        writeBucketUiState(activeRunLogBucket, { clarify_questions: [] });
        clearUnattendedAutoResumeTimer();
        armManualRunStopTimer();
        renderUnattendedState();
        setRunState('running', txt('run_running', ''));
    } catch (e) {
        resetRunSessionUiState();
        resetStopTimerState();
        renderUnattendedState();
        setStatus(`${txt('status_start_failed_prefix', '')}${e}`, 'status-danger');
    }
}

async function resumeSession(opts) {
    const options = opts || {};
    const isAuto = !!options.auto;
    try {
        ensureWorkspaceForCurrentProject();
        if (isReadOnlyView()) {
            setStatus(txt('status_readonly_view', ''), 'status-danger');
            return;
        }
        const selectedProjectId = document.getElementById('project_selector')?.value || '';
        if (!selectedProjectId) {
            setStatus(txt('status_need_select_project', ''), 'status-danger');
            return;
        }
        if (!(await runtimeSyncCall('syncSharedConfigForRun', 'status_shared_config_sync_failed'))) {
            return;
        }
        const body = await runtimeSyncCall('prepareFormDataWithWorkspace', 'status_resume_alloc_workspace_failed');
        if (!body) {
            return;
        }
        runtimeSyncCall('primeRunRequestContext');
        const resp = await fetch('/resume', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                workspace: body.workspace,
                project_id: selectedProjectId,
                unattended_mode: !!body.unattended_mode,
            }),
        });
        if (!resp.ok) {
            const msg = await resp.text();
            throw new Error(`${resp.status} ${msg}`);
        }
        setStatus(txt('status_resumed', ''), 'status-warn');
        markRunSessionStarted(activeRunProjectLabel, true);
        clearUnattendedAutoResumeTimer();
        if (!isAuto) {
            activeRunUnattendedMode = !!body.unattended_mode;
            unattendedAutoResumeRemaining = autoResumeAttemptsSetting();
        } else {
            unattendedAutoResumeRemaining = autoResumeAttemptsSetting();
        }
        writeBucketUiState(activeRunLogBucket, { clarify_questions: [] });
        const mins = stopAfterMinutesSetting();
        if (mins > 0 && manualRunStopDeadlineMs <= 0) {
            manualRunStopDeadlineMs = Date.now() + mins * 60 * 1000;
        }
        manualRunStopExpired = manualRunStopDeadlineMs > 0 && Date.now() >= manualRunStopDeadlineMs;
        renderUnattendedState();
        setRunState('running', txt('run_running', ''));
    } catch (e) {
        resetRunSessionUiState();
        setStatus(`${txt('status_resume_failed_prefix', '')}${e}`, 'status-danger');
        if (isAuto) {
            scheduleUnattendedAutoResume();
        }
        renderUnattendedState();
    }
}

	async function stopSession() {
	    try {
	        const resp = await fetch('/stop', { method: 'POST' });
	        const ack = (await resp.text()).trim();
	        if (!resp.ok) throw new Error(`stop error ${resp.status}`);
	        // User preference: treat a successful /stop call as "stopped" immediately,
	        // without waiting for a completion callback and without ack-timeout UI.
	        stopRequested = true;
	        stopDisplayedAsStopped = true;
	        runtimeSyncCall('clearStopAckTimer');
	        // Keep a minimal status hint; unlock the UI right away.
	        resetUnattendedRunState();
	        resetRunSessionUiState();
	        setRunActionButtons(false);
	        setRunState('idle', txt('run_idle', ''));
	        setStatus(txt('status_stop_backend_done', ''), 'status-warn');
	        void ack; // stop response is informational only; UI already finalized as stopped.
	    } catch (e) {
	        applyInterruptedTerminalState({
	            runTextKey: 'run_interrupted',
	            statusKey: 'status_stop_failed',
            statusClass: 'status-danger',
            message: String(e),
        });
    }
}

async function revertLast() {
    if (isReadOnlyView()) {
        setStatus(txt('status_readonly_no_revert', ''), 'status-danger');
        return;
    }
    try {
        const resp = await fetch('/revert', { method: 'POST' });
        if (!resp.ok) throw new Error(`revert error ${resp.status}`);
        appendLog(txt('log_revert_sent', ''));
    } catch (e) {
        setStatus(`${txt('status_revert_failed_prefix', '')}${e}`, 'status-danger');
    }
}

async function pushRemote() {
    if (isReadOnlyView()) {
        setStatus(txt('status_readonly_no_push', ''), 'status-danger');
        return;
    }
    const body = getFormData();
    if (!body.remote || !body.remote_url || !body.branch) {
        setStatus(txt('status_push_fields_required', ''), 'status-danger');
        return;
    }
    try {
        const resp = await fetch('/push', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                remote: body.remote,
                url: body.remote_url,
                branch: body.branch,
            }),
        });
        if (!resp.ok) throw new Error(`push error ${resp.status}`);
        setStatus(txt('status_push_sent', ''), 'status-warn');
        appendLog(txt('log_push_sent', '').replace('{remote}', body.remote).replace('{branch}', body.branch));
    } catch (e) {
        setStatus(`${txt('status_push_failed_prefix', '')}${e}`, 'status-danger');
    }
}

async function submitClarify(e) {
    if (e) e.preventDefault();
    if (isReadOnlyView()) {
        setStatus(txt('status_readonly_no_clarify', ''), 'status-danger');
        return;
    }
    const form = document.getElementById('clarify_form');
    const formData = new FormData(form);
    const answers = {};
    formData.forEach((value, key) => {
        if (answers[key]) {
            if (!Array.isArray(answers[key])) {
                answers[key] = [answers[key]];
            }
            answers[key].push(value);
        } else {
            answers[key] = value;
        }
    });
    try {
        const resp = await fetch('/clarify', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ answers }),
        });
        if (!resp.ok) throw new Error(`clarify error ${resp.status}`);
        form.dataset.locked = '0';
        writeBucketUiState(currentLogBucket(), { clarify_questions: [] });
        setRunState('running', txt('run_running', ''));
        setStatus(txt('status_clarify_submitted_waiting', ''), 'status-warn');
        appendLog(txt('log_submit_clarify', ''));
        if (viewLogBucket() === currentLogBucket()) {
            renderClarifyQuestions([], currentLogBucket(), true);
            renderUiForViewBucket();
        }
    } catch (err) {
        setStatus(`${txt('status_clarify_failed_prefix', '')}${err}`, 'status-danger');
    }
}

function renderDiff(diffText) {
    void diffText;
}

function escapeHtml(text) {
    return text
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;');
}

function bindAutoSave() {
    const hasProjectDraftContext = () => {
        const selectedId = document.getElementById('project_selector')?.value || '';
        if (selectedId) return true;
        const goal = (document.getElementById('goal')?.value || '').trim();
        const name = (document.getElementById('project_name')?.value || '').trim();
        return !!(goal || name);
    };
    const touchProjectDraft = () => {
        if (!hasProjectDraftContext()) {
            markProjectClean();
            return;
        }
        markProjectDirty();
        scheduleDraftSave();
    };
    const ids = ['unattended_mode', 'goal', 'remote', 'remote_url', 'branch', 'git_user_name', 'git_user_email'];
    ids.forEach(id => {
        const el = document.getElementById(id);
        if (!el) return;
        el.addEventListener('input', () => {
            touchProjectDraft();
            if (id === 'unattended_mode') renderUnattendedState();
        });
        el.addEventListener('change', () => {
            touchProjectDraft();
            if (id === 'unattended_mode') renderUnattendedState();
        });
    });
    const goal = document.getElementById('goal');
    if (goal) {
        goal.addEventListener('input', () => {
            autoFillProjectNameFromGoal(false);
        });
        goal.addEventListener('change', () => {
            autoFillProjectNameFromGoal(false);
        });
    }
}

function bindEvents() {
    document.getElementById('start_btn').addEventListener('click', startSession);
    document.getElementById('resume_btn').addEventListener('click', resumeSession);
    document.getElementById('stop_btn').addEventListener('click', stopSession);
    document.getElementById('go_running_project_btn').addEventListener('click', goToRunningProjectView);
    document.getElementById('revert_btn').addEventListener('click', revertLast);
    document.getElementById('push_btn').addEventListener('click', pushRemote);
    document.getElementById('clarify_submit_btn').addEventListener('click', submitClarify);
    document.getElementById('clear_log_btn').addEventListener('click', () => {
        if (isReadOnlyView()) {
            setStatus(txt('status_readonly_no_clear_log', ''), 'status-danger');
            return;
        }
        writeLogForBucket(viewLogBucket(), '');
        renderCurrentLogView();
    });
    document.getElementById('export_log_btn').addEventListener('click', exportLogs);
    document.getElementById('export_snapshot_btn').addEventListener('click', exportSnapshot);
    document.getElementById('export_report_btn').addEventListener('click', exportReleaseReport);
    document.getElementById('view_diff_btn').addEventListener('click', () => {
        const bucket = currentLogBucket();
        const url = `/diff?bucket=${encodeURIComponent(bucket)}`;
        const w = window.open(url, '_blank', 'noopener');
        if (!w) {
            setStatus(txt('status_open_diff_failed', ''), 'status-danger');
        }
    });
    document.getElementById('project_new_btn').addEventListener('click', createNewProject);
    document.getElementById('project_save_btn').addEventListener('click', () => { saveCurrentProject(); });
    document.getElementById('project_load_btn').addEventListener('click', loadSelectedProject);
    document.getElementById('project_delete_btn').addEventListener('click', () => { deleteSelectedProject(); });
    document.getElementById('project_selector').addEventListener('change', syncProjectNameFromSelection);
    document.getElementById('project_search').addEventListener('input', () => {
        projectSearchKeyword = document.getElementById('project_search').value || '';
        renderProjectAccordion();
    });
    const clarifyForm = document.getElementById('clarify_form');
    if (clarifyForm) {
        const lockClarifyRender = () => {
            if (!clarifyForm.children.length) return;
            clarifyForm.dataset.locked = '1';
        };
        clarifyForm.addEventListener('input', lockClarifyRender);
        clarifyForm.addEventListener('change', lockClarifyRender);
    }
    document.getElementById('project_name').addEventListener('input', () => {
        touchProjectDraft();
        const value = document.getElementById('project_name').value.trim();
        if (!value) {
            projectNameManualOverride = false;
            autoFillProjectNameFromGoal(false);
            return;
        }
        projectNameManualOverride = value !== lastAutoProjectName;
    });
    document.getElementById('open_global_config_btn').addEventListener('click', openGlobalConfigModal);
    document.getElementById('save_global_config_btn').addEventListener('click', saveGlobalConfig);
    document.getElementById('close_global_config_btn').addEventListener('click', closeGlobalConfigModal);
    document.getElementById('api_key').addEventListener('input', () => {
        updateSharedConfigFromInputs();
        scheduleUiCacheSave();
    });
    bindGlobalOptionInput('global_auto_resume_attempts');
    bindGlobalOptionInput('global_stop_after_minutes');
    bindGlobalOptionInput('global_history_max_messages');
    bindGlobalOptionInput('global_history_max_chars');
    bindGlobalOptionInput('global_log_max_chars', () => {
        maybeWarnLargeCharLimit(txt('label_global_log_max_chars', ''));
        renderCurrentLogView();
    });
    document.getElementById('language_select').addEventListener('change', async () => {
        const next = normalizeLanguageCode(document.getElementById('language_select').value);
        if (!next || next === currentLanguage) return;
        const ok = await switchLanguage(next, true);
        if (!ok) {
            setStatus(fmt('status_language_switch_failed', '', { code: next }), 'status-danger');
            renderLanguageOptions();
        }
    });
    document.getElementById('global_config_modal').addEventListener('click', (e) => {
        if (e.target && e.target.id === 'global_config_modal') closeGlobalConfigModal();
    });
    document.getElementById('sidebar_toggle_btn').addEventListener('click', () => {
        if (isNarrowViewport()) {
            setSidebarMobileOpen(!sidebarMobileOpen);
        } else {
            setSidebarCollapsed(!sidebarCollapsed);
        }
    });
    document.getElementById('sidebar_overlay').addEventListener('click', () => {
        setSidebarMobileOpen(false);
    });
    window.addEventListener('resize', () => {
        if (!isNarrowViewport()) {
            sidebarMobileOpen = false;
        }
        applySidebarLayout();
        // Recompute topbar marquee thresholds when layout changes.
        if (typeof refreshTopbarCombined === 'function') {
            refreshTopbarCombined();
        } else {
            const el = document.getElementById('topbar_line_text');
            if (el && typeof setTopbarHintText === 'function') {
                setTopbarHintText(el, el.textContent || '', { force: true, allowResetWhenSameText: true });
            }
        }
    });
}

function setHiddenById(id, hidden) {
    const el = document.getElementById(id);
    if (!el) return;
    el.style.display = hidden ? 'none' : '';
}

function applyNoviceUi(me, isGuest) {
    const isAdmin = !!(me && me.is_admin);
    setHiddenById('advanced_git_section', true);
    setHiddenById('advanced_unattended_section', true);
    setHiddenById('project_save_btn', true);
    setHiddenById('project_load_btn', true);
    setHiddenById('project_delete_btn', true);
    setHiddenById('vm_card', isGuest || !isAdmin);
}

function autoOpenLatestProject() {
    const items = loadProjects();
    if (!items.length) return;
    const current = document.getElementById('project_selector')?.value || '';
    if (current) return;
    const latest = items
        .slice()
        .sort((a, b) => Number(b?.updated_at || 0) - Number(a?.updated_at || 0))[0];
    if (!latest || !latest.id) return;
    const sel = document.getElementById('project_selector');
    if (!sel) return;
    sel.value = latest.id;
    loadSelectedProject(true);
    setAutoSaveState(fmt('autosave_project_restored', '', { name: latest.name || latest.id }));
}

function exportLogs() {
    const content = document.getElementById('log').textContent || '';
    const blob = new Blob([content], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${txt('download_log_prefix', '')}-${Date.now()}.txt`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
}

async function exportSnapshot() {
    try {
        const [mResp, sResp] = await Promise.all([fetch('/metrics'), fetch('/ui_state')]);
        const metrics = mResp.ok ? await mResp.json() : { error: `metrics ${mResp.status}` };
        const uiState = sResp.ok ? await sResp.json() : { error: `ui_state ${sResp.status}` };
        const snapshot = {
            exported_at: new Date().toISOString(),
            status_text: document.getElementById('status').textContent,
            diagnostics_text: document.getElementById('diagnostics').textContent,
            metrics,
            ui_state: uiState,
        };
        const blob = new Blob([JSON.stringify(snapshot, null, 2)], { type: 'application/json;charset=utf-8' });
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = `${txt('download_snapshot_prefix', '')}-${Date.now()}.json`;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
    } catch (_e) {
        setStatus(txt('status_export_snapshot_failed', ''), 'status-danger');
    }
}

async function exportReleaseReport() {
    try {
        const [mResp, sResp] = await Promise.all([fetch('/metrics'), fetch('/ui_state')]);
        const metrics = mResp.ok ? await mResp.json() : { error: `metrics ${mResp.status}` };
        const uiState = sResp.ok ? await sResp.json() : { error: `ui_state ${sResp.status}` };
        const lines = [];
        lines.push(txt('report_title', ''));
        lines.push('');
        lines.push(fmt('report_exported_at', '', { time: new Date().toISOString() }));
        lines.push(fmt('report_status', '', { status: document.getElementById('status').textContent }));
        lines.push(fmt('report_readiness', '', { readiness: (metrics.readiness || 'idle').toUpperCase() }));
        lines.push(fmt('report_score', '', { score: metrics.readiness_score ?? '-' }));
        lines.push('');
        lines.push(txt('report_key_metrics_title', ''));
        lines.push(`- events: ${metrics.total_events ?? '-'}`);
        lines.push(`- logs: ${metrics.total_logs ?? '-'}`);
        lines.push(`- done_ok/done_fail: ${metrics.total_done_ok ?? '-'}/${metrics.total_done_fail ?? '-'}`);
        lines.push(`- 5m success: ${metrics.done_5m_success_rate == null ? '-' : metrics.done_5m_success_rate.toFixed(1) + '%'}`);
        lines.push(`- api p50/p95: ${metrics.api_p50_ms ?? '-'}ms/${metrics.api_p95_ms ?? '-'}ms`);
        lines.push(`- eval p50/p95: ${metrics.eval_p50_ms ?? '-'}ms/${metrics.eval_p95_ms ?? '-'}ms`);
        lines.push('');
        const blockers = metrics.blockers || [];
        const actions = metrics.actions || [];
        lines.push(txt('report_blockers_title', ''));
        blockers.forEach(x => lines.push(`- ${x}`));
        if (blockers.length === 0) lines.push(txt('report_none_bullet', ''));
        lines.push('');
        lines.push(txt('report_actions_title', ''));
        actions.forEach(x => lines.push(`- ${x}`));
        lines.push('');
        lines.push(txt('report_digest_title', ''));
        (metrics.digest_5m || []).forEach(x => lines.push(`- ${x.category}: ${x.count}`));
        lines.push('');
        lines.push(txt('report_root_causes_title', ''));
        (metrics.root_causes_5m || []).forEach(x => lines.push(`- ${x.category}: ${x.count}`));
        lines.push('');
        lines.push(txt('report_ui_state_title', ''));
        lines.push('```json');
        lines.push(JSON.stringify(uiState, null, 2));
        lines.push('```');
        const blob = new Blob([lines.join('\n')], { type: 'text/markdown;charset=utf-8' });
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = `${txt('download_report_prefix', '')}-${Date.now()}.md`;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
    } catch (_e) {
        setStatus(txt('status_export_report_failed', ''), 'status-danger');
    }
}

async function init() {
    const me = await (window.KACF && window.KACF.auth && window.KACF.auth.ensureAuthForApp
        ? window.KACF.auth.ensureAuthForApp()
        : Promise.resolve(null));
    const isGuest = !!(window.KACF && window.KACF.auth && window.KACF.auth.isGuestMode && window.KACF.auth.isGuestMode());
    if (!isGuest) {
        await fetchUiCacheFromServer(); // loads sharedConfig.language for initLanguagePack
    }
    await initLanguagePack();
    try {
        sidebarCollapsed = localStorage.getItem('kacf_sidebar_collapsed') === '1';
    } catch (_e) {
        sidebarCollapsed = false;
    }
    sidebarMobileOpen = false;
    applyStaticCopyToDom();
    applyNoviceUi(me, isGuest);
    await initVmPanel({ guestMode: isGuest });
    try { window.KACF.auth && window.KACF.auth.renderAccountMenu && window.KACF.auth.renderAccountMenu(); } catch (_e) {}
    if (me && me.forced_notice) {
        await showForcedNoticeModal(String(me.forced_notice || ''), Number(me.forced_notice_min_seconds || 0));
    }

    if (isGuest) {
        // Guest mode: read-only with sample content, no backend realtime channels.
        renderGuestDemo();
        bindEvents();
        setProjectControlsDisabled(true);
        setConfigInputsDisabled(true);
        applyReadOnlyMode();
        return;
    }
    applySharedConfigToInputs();
    applyGlobalOptionsToInputs();
    await refreshProjectsFromServer();
    renderProjectSelector('');
    bindEvents();
    bindAutoSave();
    setProjectDraftState();
    setRunningProjectIndicator(txt('running_project_none', ''));
    setProjectControlsDisabled(false);
    applyReadOnlyMode();
    updateGoRunningProjectButton();
    setRunActionButtons(false);
    document.getElementById('revert_btn').disabled = true;
    setRunState('idle', txt('run_idle', ''));
    setAutoSaveState(fmt('autosave_project_idle', '', {}));
    renderUnattendedState();
    autoFillProjectNameFromGoal(false);
    markProjectClean();
    applySharedConfigToInputs();
    autoOpenLatestProject();
    resetBucketRuntimeUiState(viewLogBucket());
    renderCurrentLogView();
    renderUiForViewBucket();
    await loadProjectConfigForWorkspace();
    await runtimeSyncCall('refreshUiState');
    await runtimeSyncCall('refreshMetrics');
    await runtimeSyncCall('bootstrapEventCursor');
    renderUnattendedState();
    renderUiForViewBucket();
    setInterval(() => runtimeSyncCall('refreshUiState'), UI_STATE_SYNC_MS);
    setInterval(() => runtimeSyncCall('refreshMetrics'), 5000);
    setInterval(renderUnattendedState, 1000);
    setInterval(() => runtimeSyncCall('monitorRealtimeChannel'), 2000);
    // Prefer SSE for low-latency updates, fallback to long-polling only on failure.
    runtimeSyncCall('startEventStream');
    window.addEventListener('beforeunload', () =>
        runtimeSyncCall('closeEventStream')
    );
}

async function showForcedNoticeModal(message, minSeconds) {
    const msg = String(message || '').trim();
    if (!msg) return;
    const secs = Math.max(0, Math.min(300, Number(minSeconds || 0)));
    const overlay = document.createElement('div');
    overlay.style.position = 'fixed';
    overlay.style.left = '0';
    overlay.style.top = '0';
    overlay.style.right = '0';
    overlay.style.bottom = '0';
    overlay.style.zIndex = '80';
    overlay.style.background = 'rgba(0,0,0,0.55)';
    overlay.style.display = 'flex';
    overlay.style.alignItems = 'center';
    overlay.style.justifyContent = 'center';
    overlay.style.padding = '18px';

    const card = document.createElement('div');
    card.className = 'card';
    card.style.maxWidth = '720px';
    card.style.width = '100%';
    card.style.margin = '0';

    const h = document.createElement('h2');
    h.className = 'title';
    h.textContent = txt('notice_title', 'Notice');
    card.appendChild(h);

    const p = document.createElement('pre');
    p.className = 'codeblock';
    p.style.whiteSpace = 'pre-wrap';
    p.textContent = msg;
    card.appendChild(p);

    const hint = document.createElement('div');
    hint.className = 'hint';
    hint.style.marginTop = '10px';
    card.appendChild(hint);

    const btnRow = document.createElement('div');
    btnRow.className = 'buttons';
    btnRow.style.marginTop = '10px';
    const closeBtn = document.createElement('button');
    closeBtn.className = 'btn-primary';
    closeBtn.textContent = txt('notice_close', 'Close');
    closeBtn.disabled = secs > 0;
    btnRow.appendChild(closeBtn);
    card.appendChild(btnRow);

    overlay.appendChild(card);
    document.body.appendChild(overlay);

    let left = secs;
    const tick = () => {
        if (left <= 0) {
            hint.textContent = '';
            closeBtn.disabled = false;
            return;
        }
        hint.textContent = fmt('notice_wait_left', 'Please wait {seconds}s...', { seconds: left });
        left -= 1;
        setTimeout(tick, 1000);
    };
    tick();

    closeBtn.addEventListener('click', () => {
        try { document.body.removeChild(overlay); } catch (_e) {}
    });
}

function renderGuestDemo() {
    // Minimal demo content: keep UI stable and clearly non-destructive.
    try {
        document.getElementById('project_name').value = txt('project_default_name', 'Demo Project').replace('{index}', '1');
        document.getElementById('goal').value = txt('guest_demo_goal', 'Guest mode demo: all inputs are read-only. Login to create and run projects.');
        document.getElementById('remote_url').value = 'https://example.com/repo.git';
        document.getElementById('workspace').value = '';
        setStatus(txt('readonly_guest_mode', ''), 'status-warn');
        appendLog(txt('banner_guest_mode', ''));
        renderProjectAccordion();
        renderCurrentLogView();
        renderUiForViewBucket();
    } catch (_e) {}
}

init();
})();
