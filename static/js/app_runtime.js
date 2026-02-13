function renderResumeBanner(info) {
    const banner = document.getElementById('resume_banner');
    if (!info || !info.resume) {
        banner.style.display = 'none';
        return;
    }
    const r = info.resume;
    const timeText = r.updated_at_unix ? new Date(r.updated_at_unix * 1000).toLocaleString() : txt('unknown_time', '');
    const status = r.last_status || txt('unknown_status', '');
    const text = fmt(r.resumable ? 'resume_banner_yes' : 'resume_banner_no', '', {
        time: timeText,
        iteration: r.iteration || '?',
        status,
    });
    banner.textContent = text;
    banner.style.display = 'block';
}

function syncManualStopDeadlineFromRuntime(runtime) {
    const mins = stopAfterMinutesSetting();
    const serverStartMs = Number(runtime?.last_start_unix || 0) * 1000;
    if (mins > 0 && serverStartMs > 0) {
        manualRunStopDeadlineMs = serverStartMs + mins * 60 * 1000;
        manualRunStopExpired = Date.now() >= manualRunStopDeadlineMs;
    } else if (mins > 0 && manualRunStopDeadlineMs <= 0) {
        manualRunStopDeadlineMs = Date.now() + mins * 60 * 1000;
    }
}

function syncRunningUiFromRuntime(runtime) {
    syncManualStopDeadlineFromRuntime(runtime);
    const label = (runtime?.last_workspace || '').trim() || txt('running_backend_task', '');
    if (!activeRunLogBucket) {
        activeRunLogBucket = `runtime:${label}`;
    }
    unattendedAutoResumeRemaining = autoResumeAttemptsSetting();
    markRunSessionStarted(label, false);
    if (hasPendingClarify(currentLogBucket())) {
        setStatus(txt('status_need_clarify', ''), 'status-warn');
        setRunState('waiting_clarify', txt('run_waiting_clarify', ''));
        return;
    }
    setStatus(txt('status_running_detected', ''), 'status-warn');
    setRunState('running', txt('run_running', ''));
}

function syncStoppedUiFromRuntime() {
    syncStoppedStateIfNeeded(txt('sync_reason_backend_stopped', ''));
    resetBucketRuntimeUiState(currentLogBucket());
    if (viewLogBucket() === currentLogBucket()) {
        renderClarifyQuestions([], currentLogBucket(), true);
    }
    resetRunSessionUiState();
}

async function syncSharedConfigForRun(syncFailStatusKey) {
    const synced = await persistSharedConfigNow();
    if (synced) return true;
    setStatus(txt(syncFailStatusKey, ''), 'status-danger');
    return false;
}

async function prepareFormDataWithWorkspace(allocWorkspaceFailStatusKey) {
    let body = getFormData();
    if (body.workspace) return body;
    const ok = await saveCurrentProject();
    ensureWorkspaceForCurrentProject();
    body = getFormData();
    if (ok && body.workspace) return body;
    setStatus(txt(allocWorkspaceFailStatusKey, ''), 'status-danger');
    return null;
}

function primeRunRequestContext() {
    activeRunLogBucket = currentLogBucket();
    activeRunProjectLabel = currentProjectLabel();
}

async function refreshUiState() {
    try {
        const resp = await fetch('/ui_state');
        if (!resp.ok) return;
        markBackendAlive();
        const data = await resp.json();
        renderResumeBanner(data);
        if (data.runtime && data.runtime.running) {
            syncRunningUiFromRuntime(data.runtime);
        } else {
            syncStoppedUiFromRuntime();
        }
    } catch (_e) {
        markBackendFailure('ui_state');
    }
}

async function refreshMetrics() {
    try {
        const resp = await fetch('/metrics');
        if (!resp.ok) return;
        markBackendAlive();
        const m = await resp.json();
        const success5 = m.done_5m_success_rate == null ? '-' : `${m.done_5m_success_rate.toFixed(1)}%`;
        const trend = (m.success_rate_series_5m || []).map(p => `-${p.minute_ago}m:${p.success_rate == null ? '-' : p.success_rate.toFixed(0) + '%'}`).join(' ');
        const gateText = `gate=${m.gate_passed ? 'PASS' : 'FAIL'}@${m.gate_threshold ?? '-'} (${m.gate_reason || '-'})`;
        document.getElementById('metrics_text').textContent = fmt('metrics_line', '', {
            score: m.readiness_score ?? '-',
            gate: gateText,
            events: m.total_events,
            logs: m.total_logs,
            done_ok: m.total_done_ok,
            done_fail: m.total_done_fail,
            buffer: m.event_buffer_len,
            done_5m_ok: m.done_5m_ok,
            done_5m_fail: m.done_5m_fail,
            success5,
            trend,
            api_p50_ms: m.api_p50_ms ?? '-',
            api_p95_ms: m.api_p95_ms ?? '-',
            eval_p50_ms: m.eval_p50_ms ?? '-',
            eval_p95_ms: m.eval_p95_ms ?? '-',
        });
        const dist = (m.digest_5m || []).map(x => `${x.category}:${x.count}`).join(', ') || txt('none_text', '');
        document.getElementById('digest_dist_text').textContent = fmt('digest_dist_line', '', { dist });
        const rootTop = (m.root_causes_5m || []).map(x => `${x.category}:${x.count}`).join(' | ') || txt('none_text', '');
        document.getElementById('root_top_text').textContent = fmt('root_top_line', '', { top: rootTop });
        if (!m.running) {
            syncStoppedStateIfNeeded('metrics.running=false');
        }
        const chip = document.getElementById('readiness_chip');
        const level = (m.readiness || 'idle').toLowerCase();
        chip.className = `readiness ${level}`;
        chip.textContent = fmt('readiness_line', '', { level: level.toUpperCase() });
    } catch (_e) {
        markBackendFailure('metrics');
    }
}

async function bootstrapEventCursor() {
    try {
        const resp = await fetch('/metrics');
        if (!resp.ok) return;
        const m = await resp.json();
        const nextId = Number(m?.next_event_id || 0);
        if (Number.isFinite(nextId) && nextId > 0) {
            lastEventId = Math.max(lastEventId, nextId);
        }
    } catch (_e) {
        // Keep default cursor when backend metrics is temporarily unavailable.
    }
}

async function pollEvents() {
    if (!pollingStarted) return;
    if (polling) return;
    polling = true;
    try {
        const resp = await fetch(`/events?from=${lastEventId}`);
        if (!resp.ok) throw new Error(`events error ${resp.status}`);
        markBackendAlive();
        const events = await resp.json();
        events.forEach(([id, evt]) => {
            lastEventId = Math.max(lastEventId, id + 1);
            handleEvent(evt);
        });
    } catch (_e) {
        markBackendFailure('events');
    } finally {
        polling = false;
    }
    if (pollingStarted) {
        pollTimer = setTimeout(pollEvents, document.hidden ? POLL_MS_IDLE : POLL_MS_ACTIVE);
    } else {
        pollTimer = null;
    }
}

function closeEventStream() {
    if (eventSource) {
        eventSource.close();
        eventSource = null;
    }
    lastSseMessageAt = 0;
    sseConnectStartedAt = 0;
}

function clearStopAckTimer() {
    if (stopAckTimer) {
        clearTimeout(stopAckTimer);
        stopAckTimer = null;
    }
}

function ensurePollingFallback(reason) {
    if (pollingStarted) return;
    pollingStarted = true;
    const now = Date.now();
    if (reason !== lastRealtimeFallbackReason || (now - lastRealtimeFallbackAt) > FALLBACK_LOG_DEDUP_MS) {
        appendLog(fmt('log_realtime_fallback', '', { reason }));
        lastRealtimeFallbackReason = reason;
        lastRealtimeFallbackAt = now;
    }
    pollEvents();
}

function startEventStream(force) {
    const restart = !!force;
    if (!window.EventSource) {
        ensurePollingFallback(txt('fallback_no_eventsource', ''));
        return;
    }
    if (!restart && eventSource && (eventSource.readyState === EventSource.OPEN || eventSource.readyState === EventSource.CONNECTING)) {
        return;
    }
    closeEventStream();
    eventSource = new EventSource(`/events/stream?from=${lastEventId}`);
    sseConnectStartedAt = Date.now();
    eventSource.onopen = () => {
        markBackendAlive();
        sseErrorCount = 0;
        lastSseMessageAt = Date.now();
        if (pollingStarted) {
            pollingStarted = false;
            if (pollTimer) {
                clearTimeout(pollTimer);
                pollTimer = null;
            }
            appendLog(txt('log_realtime_restored', ''));
        }
        appendLog(txt('log_realtime_connected', ''));
    };
    eventSource.onmessage = (event) => {
        markBackendAlive();
        lastSseMessageAt = Date.now();
        try {
            const data = JSON.parse(event.data || '{}');
            if (data.heartbeat) return;
            const id = Number(data.id || 0);
            const evt = data.evt;
            if (!evt || Number.isNaN(id)) return;
            lastEventId = Math.max(lastEventId, id + 1);
            handleEvent(evt);
        } catch (_e) {
            // Keep polling alive to avoid losing progress updates on malformed chunks.
        }
    };
    eventSource.onerror = () => {
        markBackendFailure('SSE');
        sseErrorCount += 1;
        const waitMs = sseConnectStartedAt ? Date.now() - sseConnectStartedAt : 0;
        if (!pollingStarted && sseErrorCount >= SSE_ERROR_LIMIT && waitMs >= SSE_CONNECT_GRACE_MS) {
            ensurePollingFallback(txt('fallback_sse_fail', ''));
        }
    };
}

function monitorRealtimeChannel() {
    checkBackendHealth();
    if (backendOffline) return;
    if (!eventSource) {
        startEventStream();
        return;
    }
    if (eventSource.readyState === EventSource.CLOSED) {
        appendLog(txt('log_realtime_closed_reconnect', ''));
        startEventStream(true);
        return;
    }
    if (!lastSseMessageAt) return;
    if (Date.now() - lastSseMessageAt <= SSE_STALE_MS) return;
    appendLog(txt('log_realtime_stale_reconnect', ''));
    startEventStream(true);
}

function handleLogEvent(evt) {
    if (currentRunState() === 'waiting_clarify' && !hasPendingClarify(currentLogBucket())) {
        setRunState('running', txt('run_running', ''));
    }
    appendLog(evt.line);
    maybeStopOnRoundBoundary(evt.line);
    updateDiagnosticsFromLog(evt.line);
}

function handleDiffEvent(evt) {
    renderDiff(evt.diff);
}

function handleNeedClarifyEvent(evt) {
    if (activeRunUnattendedMode) {
        appendLog(txt('log_unattended_clarify_ignored', ''));
        writeBucketUiState(currentLogBucket(), { clarify_questions: [] });
        if (viewLogBucket() === currentLogBucket()) renderUiForViewBucket();
        return;
    }
    showClarify(evt.questions);
}

function handleDoneEvent(evt) {
    const wasStopRequested = stopRequested;
    const outcome = evt.success
        ? 'success'
        : ((wasStopRequested || isInterruptedMessage(evt.message)) ? 'interrupted' : 'failed');
    finishRunSessionUi();
    applyDoneOutcome(outcome, evt.message);
    if (!runSessionActive && unattendedAutoResumeRemaining <= 0) {
        resetStopTimerState();
        renderUnattendedState();
    }
    uiCacheLastHeavySyncMs = 0;
    scheduleUiCacheSave();
}

const EVENT_HANDLERS = {
    log: handleLogEvent,
    diff: handleDiffEvent,
    need_clarify: handleNeedClarifyEvent,
    done: handleDoneEvent,
};

function handleEvent(evt) {
    const handler = EVENT_HANDLERS[evt?.type];
    if (!handler) return;
    handler(evt);
}

function showClarify(questions) {
    const bucket = currentLogBucket();
    if (activeRunUnattendedMode || !runSessionActive) {
        writeBucketUiState(bucket, { clarify_questions: [] });
        if (viewLogBucket() === bucket) renderClarifyQuestions([], bucket, true);
        return;
    }
    const list = Array.isArray(questions) ? questions : [];
    writeBucketUiState(bucket, { clarify_questions: list });
    if (list.length > 0) {
        setStatus(txt('status_need_clarify', ''), 'status-warn');
        setRunState('waiting_clarify', txt('run_waiting_clarify', ''));
        appendLog(txt('log_need_clarify', ''));
    }
    if (viewLogBucket() === bucket) renderUiForViewBucket();
}

async function startSession() {
    ensureWorkspaceForCurrentProject();
    if (isReadOnlyView()) {
        setStatus(txt('status_readonly_view', ''), 'status-danger');
        return;
    }
    if (!(await syncSharedConfigForRun('status_start_shared_config_sync_failed'))) {
        return;
    }
    const body = await prepareFormDataWithWorkspace('status_start_alloc_workspace_failed');
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
        primeRunRequestContext();
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
        if (!(await syncSharedConfigForRun('status_shared_config_sync_failed'))) {
            return;
        }
        const body = await prepareFormDataWithWorkspace('status_resume_alloc_workspace_failed');
        if (!body) {
            return;
        }
        primeRunRequestContext();
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
        stopRequested = true;
        clearStopAckTimer();
        stopAckTimer = setTimeout(() => {
            if (!stopRequested) return;
            applyInterruptedTerminalState({
                runTextKey: 'run_interrupted_timeout',
                statusKey: 'status_stop_ack_timeout',
                statusClass: 'status-warn',
            });
        }, 6000);
        applyStopAcceptedStatus(ack);
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
    const bucket = currentLogBucket();
    const safe = sanitizeDiffText(diffText || '');
    writeBucketUiState(bucket, { diff_text: safe });
    if (viewLogBucket() !== bucket) return;
    renderDiffPanel(safe);
}

function escapeHtml(text) {
    return text
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;');
}

function bindAutoSave() {
    const ids = ['auto_revert_profile', 'precheck_cmd', 'release_gate_threshold', 'unattended_mode', 'goal', 'eval_cmd', 'success_regex', 'remote', 'remote_url', 'branch'];
    ids.forEach(id => {
        const el = document.getElementById(id);
        if (!el) return;
        el.addEventListener('input', () => {
            markProjectDirty();
            scheduleDraftSave();
            if (id === 'unattended_mode') renderUnattendedState();
        });
        el.addEventListener('change', () => {
            markProjectDirty();
            scheduleDraftSave();
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
        markProjectDirty();
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
    bindGlobalOptionInput('global_diff_max_chars', () => {
        maybeWarnLargeCharLimit(txt('label_global_diff_max_chars', ''));
        renderUiForViewBucket();
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
            setSidebarMobileOpen(false);
        } else {
            setSidebarCollapsed(!sidebarCollapsed);
        }
    });
    document.getElementById('sidebar_mobile_btn').addEventListener('click', () => {
        setSidebarMobileOpen(true);
    });
    document.getElementById('sidebar_overlay').addEventListener('click', () => {
        setSidebarMobileOpen(false);
    });
    window.addEventListener('resize', () => {
        if (!isNarrowViewport()) {
            sidebarMobileOpen = false;
        }
        applySidebarLayout();
    });
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
    await fetchUiCacheFromServer();
    await initLanguagePack();
    try {
        sidebarCollapsed = localStorage.getItem('kacf_sidebar_collapsed') === '1';
    } catch (_e) {
        sidebarCollapsed = false;
    }
    sidebarMobileOpen = false;
    applyStaticCopyToDom();
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
    await refreshUiState();
    await refreshMetrics();
    await bootstrapEventCursor();
    renderUnattendedState();
    renderUiForViewBucket();
    setInterval(refreshUiState, UI_STATE_SYNC_MS);
    setInterval(refreshMetrics, 5000);
    setInterval(renderUnattendedState, 1000);
    setInterval(monitorRealtimeChannel, 2000);
    // Prefer SSE for low-latency updates, fallback to long-polling only on failure.
    startEventStream();
    window.addEventListener('beforeunload', closeEventStream);
}

init();
