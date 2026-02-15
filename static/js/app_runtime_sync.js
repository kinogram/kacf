(() => {
function assertRuntimeSyncDependencies() {
    const root = window.KACF || {};
    const required = {
        state: ['txt', 'fmt', 'currentLogBucket', 'viewLogBucket', 'currentProjectLabel'],
        uiCache: [
            'persistSharedConfigNow',
            'renderClarifyQuestions',
            'resetBucketRuntimeUiState',
            'writeBucketUiState',
            'renderUiForViewBucket',
            'scheduleUiCacheSave',
        ],
        logPipeline: [
            'appendLog',
            'setStatus',
            'setRunState',
            'hasPendingClarify',
            'updateDiagnosticsFromLog',
            'markBackendAlive',
            'markBackendFailure',
        ],
        runtimeState: [
            'stopAfterMinutesSetting',
            'autoResumeAttemptsSetting',
            'markRunSessionStarted',
            'syncStoppedStateIfNeeded',
            'checkBackendHealth',
            'isInterruptedMessage',
            'finishRunSessionUi',
            'applyDoneOutcome',
            'resetStopTimerState',
            'renderUnattendedState',
            'getFormData',
        ],
        projects: ['saveCurrentProject', 'ensureWorkspaceForCurrentProject'],
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

assertRuntimeSyncDependencies();

const {
    txt,
    fmt,
    currentLogBucket,
    viewLogBucket,
    currentProjectLabel,
} = window.KACF.state;
const {
    persistSharedConfigNow,
    renderClarifyQuestions,
    resetBucketRuntimeUiState,
    writeBucketUiState,
    renderUiForViewBucket,
    scheduleUiCacheSave,
} = window.KACF.uiCache;
const {
    appendLog,
    setStatus,
    setRunState,
    hasPendingClarify,
    updateDiagnosticsFromLog,
    markBackendAlive,
    markBackendFailure,
} = window.KACF.logPipeline;
const {
    stopAfterMinutesSetting,
    autoResumeAttemptsSetting,
    markRunSessionStarted,
    syncStoppedStateIfNeeded,
    checkBackendHealth,
    isInterruptedMessage,
    finishRunSessionUi,
    applyDoneOutcome,
    resetStopTimerState,
    renderUnattendedState,
    getFormData,
} = window.KACF.runtimeState;
const { saveCurrentProject, ensureWorkspaceForCurrentProject } = window.KACF.projects;

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
    if (stopDisplayedAsStopped) {
        // UI already finalized as "stopped" immediately after /stop succeeded.
        // Keep it stable and just clear flags.
        stopDisplayedAsStopped = false;
        stopRequested = false;
        clearStopAckTimer();
        return;
    }
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

// Explicitly expose runtime-sync capabilities to reduce implicit cross-file coupling.
window.KACF = window.KACF || {};
window.KACF.runtimeSync = {
    syncSharedConfigForRun,
    prepareFormDataWithWorkspace,
    primeRunRequestContext,
    clearStopAckTimer,
    refreshUiState,
    refreshMetrics,
    bootstrapEventCursor,
    monitorRealtimeChannel,
    startEventStream,
    closeEventStream,
};
})();
