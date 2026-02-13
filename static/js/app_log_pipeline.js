function loadProjectLogs() {
    return cacheProjectLogs || {};
}

function saveProjectLogs(obj) {
    cacheProjectLogs = sanitizeProjectLogsMap((obj && typeof obj === 'object') ? obj : {});
    pruneLogBuckets();
    scheduleUiCacheSave();
}

function readLogForBucket(bucket) {
    const all = loadProjectLogs();
    const raw = all[bucket];
    return typeof raw === 'string' ? raw : '';
}

function writeLogForBucket(bucket, content) {
    const all = loadProjectLogs();
    all[bucket] = sanitizeLogContent(content || '');
    touchBucket(logBucketTouchedAt, bucket);
    pruneLogBuckets();
    saveProjectLogs(all);
}

function renderCurrentLogView() {
    const log = document.getElementById('log');
    log.textContent = readLogForBucket(viewLogBucket());
    log.scrollTop = log.scrollHeight;
}

function readLogLinesForBucket(bucket) {
    const raw = readLogForBucket(bucket);
    if (!raw) return [];
    const lines = raw.split('\n');
    if (lines.length && lines[lines.length - 1] === '') lines.pop();
    return lines;
}

function writeLogLinesForBucket(bucket, lines) {
    writeLogForBucket(bucket, `${lines.join('\n')}\n`);
}

function isModelProgressLine(line) {
    return String(line || '').startsWith('[Model] 仍在生成中...');
}

function trimTrailingModelProgress(lines) {
    if (lines.length && isModelProgressLine(lines[lines.length - 1])) {
        lines.pop();
    }
}

function parseModelStreamTag(line) {
    const raw = String(line || '');
    if (raw.startsWith('[Model-Stream]')) return '[Model-Stream]';
    if (raw.startsWith('[Model-Thought]')) return '[Model-Thought]';
    return '';
}

function streamBodyLines(raw, tag) {
    const body = String(raw || '').slice(tag.length).trim();
    if (!body) return [];
    const out = [];
    for (const line of body.split(/\r?\n/)) {
        const expanded = line
            .replace(/\\r/g, '\r')
            .replace(/\\n/g, '\n')
            .replace(/\\t/g, '\t');
        for (const seg of expanded.split('\n')) {
            const pretty = prettyJsonStreamLine(seg);
            for (const one of pretty) out.push(one);
        }
    }
    return out;
}

function prettyJsonStreamLine(line) {
    const raw = String(line || '');
    const trimmed = raw.trim();
    if (!trimmed) return [''];
    const startsLikeJson = trimmed.startsWith('{') || trimmed.startsWith('[');
    const endsLikeJson = trimmed.endsWith('}') || trimmed.endsWith(']');
    if (!(startsLikeJson && endsLikeJson)) return [raw];
    try {
        const parsed = JSON.parse(trimmed);
        return JSON.stringify(parsed, null, 2).split('\n');
    } catch (_e) {
        return [raw];
    }
}

function getLogRenderState(bucket) {
    if (!logRenderStateByBucket[bucket]) {
        logRenderStateByBucket[bucket] = {
            activeModelStreamTag: '',
            streamLineSinceTrim: 0,
        };
    }
    return logRenderStateByBucket[bucket];
}

function appendLog(text) {
    const line = String(text || '');
    const bucket = currentLogBucket();
    const state = getLogRenderState(bucket);
    const lines = readLogLinesForBucket(bucket);
    const streamTag = parseModelStreamTag(line);
    let appendedLines = 0;
    if (streamTag) {
        trimTrailingModelProgress(lines);
        if (state.activeModelStreamTag !== streamTag) {
            lines.push(streamTag);
            state.activeModelStreamTag = streamTag;
            appendedLines += 1;
        }
        const bodyLines = streamBodyLines(line, streamTag);
        for (const one of bodyLines) {
            lines.push(one);
            appendedLines += 1;
        }
    } else if (isModelProgressLine(line)) {
        if (lines.length && isModelProgressLine(lines[lines.length - 1])) {
            lines[lines.length - 1] = line;
        } else {
            lines.push(line);
            appendedLines += 1;
        }
    } else {
        state.activeModelStreamTag = '';
        lines.push(line);
        appendedLines += 1;
    }
    writeLogLinesForBucket(bucket, lines);
    state.streamLineSinceTrim += appendedLines;
    if (state.streamLineSinceTrim >= 10) {
        writeLogForBucket(bucket, readLogForBucket(bucket));
        state.streamLineSinceTrim = 0;
    }
    const log = document.getElementById('log');
    log.textContent = readLogForBucket(bucket);
    log.scrollTop = log.scrollHeight;
}

function updateDiagnosticsFromLog(text) {
    const m = text.match(/^\[Eval-Digest\]\s+category=([^\s]+)\s+severity=([^\s]+)\s+repeated=([^\s]+)\s+signature=(.*)$/);
    if (!m) return;
    const [, category, severity, repeated, signature] = m;
    const bucket = currentLogBucket();
    writeBucketUiState(bucket, {
        diagnostics_text: fmt('diagnostics_line', '', { category, severity, repeated, signature }),
    });
    if (viewLogBucket() === bucket) renderUiForViewBucket();
}

function setStatus(msg, cls) {
    const bucket = currentLogBucket();
    writeBucketUiState(bucket, {
        status_text: msg,
        status_class: cls || '',
    });
    if (viewLogBucket() === bucket) renderUiForViewBucket();
}

function setRunState(state, text) {
    const bucket = currentLogBucket();
    writeBucketUiState(bucket, {
        run_state: state || 'idle',
        run_text: text || txt('run_idle', ''),
    });
    if (viewLogBucket() === bucket) renderUiForViewBucket();
}

function currentRunState() {
    if (!runSessionActive) return 'idle';
    return readBucketUiState(currentLogBucket()).run_state || 'idle';
}

function hasPendingClarify(bucket) {
    if (!shouldAllowClarifyInteraction()) return false;
    const state = readBucketUiState(bucket || currentLogBucket());
    return Array.isArray(state.clarify_questions) && state.clarify_questions.length > 0;
}

function isRunStateActive(state) {
    return state === 'running' || state === 'stopping';
}

function setBackendOfflineState(nextOffline) {
    if (backendOffline === !!nextOffline) return;
    backendOffline = !!nextOffline;
    if (backendOffline) {
        closeGlobalConfigModal();
        applyReadOnlyMode();
        return;
    }
    applyReadOnlyMode();
    if (!backendReconnectReloading) {
        backendReconnectReloading = true;
        setTimeout(() => window.location.reload(), 120);
    }
}

function markBackendAlive() {
    backendFailureCount = 0;
    backendOfflineNotified = false;
    setBackendOfflineState(false);
}

window.KACF = window.KACF || {};
window.KACF.logPipeline = {
    loadProjectLogs,
    saveProjectLogs,
    readLogForBucket,
    writeLogForBucket,
    renderCurrentLogView,
    readLogLinesForBucket,
    writeLogLinesForBucket,
    isModelProgressLine,
    trimTrailingModelProgress,
    parseModelStreamTag,
    streamBodyLines,
    prettyJsonStreamLine,
    getLogRenderState,
    appendLog,
    updateDiagnosticsFromLog,
    setStatus,
    setRunState,
    currentRunState,
    hasPendingClarify,
    isRunStateActive,
    setBackendOfflineState,
    markBackendAlive,
};
