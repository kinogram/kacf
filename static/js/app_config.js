async function fetchUiCacheFromServer() {
    try {
        const resp = await fetch('/ui_cache');
        if (!resp.ok) return false;
        const data = await resp.json();
        if (data.shared_config && typeof data.shared_config === 'object') {
            sharedConfig = {
                api_key: data.shared_config.api_key || '',
                base_url: data.shared_config.base_url || 'https://api.deepseek.com',
                model: data.shared_config.model || 'deepseek-reasoner',
                language: normalizeLanguageCode(data.shared_config.language || ''),
            };
        }
        if (data.global_options && typeof data.global_options === 'object') {
            globalOptions = normalizeGlobalOptions(data.global_options);
        }
        cacheProjectLogs = sanitizeProjectLogsMap(
            (data.project_logs && typeof data.project_logs === 'object') ? data.project_logs : {}
        );
        cacheProjectUiState = sanitizeProjectUiStateMap(
            (data.project_ui_state && typeof data.project_ui_state === 'object') ? data.project_ui_state : {}
        );
        return true;
    } catch (_e) {
        return false;
    }
}

function scheduleUiCacheSave() {
    if (uiCacheSaveTimer) clearTimeout(uiCacheSaveTimer);
    uiCacheSaveTimer = setTimeout(async () => {
        const now = Date.now();
        const heavyIntervalMs = runSessionActive ? 5000 : 1000;
        const includeHeavy = (now - uiCacheLastHeavySyncMs) >= heavyIntervalMs;
        const payload = {
            shared_config: sharedConfig,
            global_options: globalOptions,
        };
        if (includeHeavy) {
            payload.project_logs = cacheProjectLogs;
            payload.project_ui_state = cacheProjectUiState;
            uiCacheLastHeavySyncMs = now;
        }
        try {
            await fetch('/ui_cache', {
                method: 'PUT',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(payload),
            });
        } catch (_e) {}
    }, 500);
}

async function persistSharedConfigNow() {
    updateSharedConfigFromInputs();
    updateGlobalOptionsFromInputs();
    try {
        const resp = await fetch('/ui_cache', {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ shared_config: sharedConfig, global_options: globalOptions }),
        });
        return resp.ok;
    } catch (_e) {
        return false;
    }
}

function updateGlobalOptionsFromInputs() {
    const next = { ...GLOBAL_OPTION_DEFAULTS };
    Object.entries(GLOBAL_OPTION_INPUT_ID_BY_KEY).forEach(([key, inputId]) => {
        const el = document.getElementById(inputId);
        next[key] = String(el?.value ?? GLOBAL_OPTION_DEFAULTS[key]).trim();
    });
    globalOptions = next;
}

function applyGlobalOptionsToInputs() {
    const normalized = normalizeGlobalOptions(globalOptions);
    globalOptions = normalized;
    Object.entries(GLOBAL_OPTION_INPUT_ID_BY_KEY).forEach(([key, inputId]) => {
        const el = document.getElementById(inputId);
        if (el) el.value = normalized[key];
    });
}

function bindGlobalOptionInput(inputId, onAfterUpdate) {
    const el = document.getElementById(inputId);
    if (!el) return;
    el.addEventListener('input', () => {
        updateGlobalOptionsFromInputs();
        if (typeof onAfterUpdate === 'function') onAfterUpdate();
        scheduleUiCacheSave();
    });
}

function loadProjectUiStateMap() {
    return cacheProjectUiState || {};
}

function saveProjectUiStateMap(obj) {
    cacheProjectUiState = (obj && typeof obj === 'object') ? obj : {};
    scheduleUiCacheSave();
}

function defaultBucketUiState() {
    return {
        status_text: txt('status_waiting', ''),
        status_class: '',
        run_state: 'idle',
        run_text: txt('run_idle', ''),
        diagnostics_text: txt('diagnostics_none', ''),
        diff_text: '',
        clarify_questions: [],
    };
}

function readBucketUiState(bucket) {
    const all = loadProjectUiStateMap();
    const raw = all[bucket];
    if (!raw || typeof raw !== 'object') return defaultBucketUiState();
    const merged = { ...defaultBucketUiState(), ...raw };
    merged.diff_text = sanitizeDiffText(merged.diff_text || '');
    return merged;
}

function writeBucketUiState(bucket, patch) {
    const all = loadProjectUiStateMap();
    const next = { ...readBucketUiState(bucket), ...(patch || {}) };
    next.diff_text = sanitizeDiffText(next.diff_text || '');
    all[bucket] = next;
    touchBucket(uiStateBucketTouchedAt, bucket);
    pruneUiStateBuckets();
    saveProjectUiStateMap(all);
}

function renderDiffPanel(diffText) {
    const safe = sanitizeDiffText(diffText || '');
    const section = document.getElementById('diff_section');
    const diffElem = document.getElementById('diff');
    if (!safe) {
        section.style.display = 'none';
        diffElem.innerHTML = '';
        return;
    }
    const lines = safe.split('\n');
    diffElem.innerHTML = lines.map(line => {
        if (line.startsWith('+') && !line.startsWith('+++')) {
            return `<div class="diff-line-add">${escapeHtml(line)}</div>`;
        }
        if (line.startsWith('-') && !line.startsWith('---')) {
            return `<div class="diff-line-del">${escapeHtml(line)}</div>`;
        }
        return `<div class="diff-line-other">${escapeHtml(line)}</div>`;
    }).join('');
    section.style.display = 'block';
}

function normalizeClarifyQuestions(questions) {
    return (Array.isArray(questions) ? questions : []).map(q => ({
        id: String(q?.id || ''),
        question: String(q?.question || ''),
        type: String(q?.type || 'text'),
        options: Array.isArray(q?.options) ? q.options.map(opt => String(opt)) : [],
    }));
}

function snapshotClarifyAnswers(form) {
    const formData = new FormData(form);
    const answers = {};
    formData.forEach((value, key) => {
        const v = String(value);
        if (Object.prototype.hasOwnProperty.call(answers, key)) {
            if (!Array.isArray(answers[key])) answers[key] = [answers[key]];
            answers[key].push(v);
            return;
        }
        answers[key] = v;
    });
    return answers;
}

function renderClarifyQuestions(questions, bucket, force) {
    const form = document.getElementById('clarify_form');
    const normalized = normalizeClarifyQuestions(questions);
    const bucketKey = String(bucket || '');
    const locked = form.dataset.locked === '1' && form.dataset.lockBucket === bucketKey;
    if (!force && normalized.length > 0 && locked) return;
    const signature = JSON.stringify(normalized);
    const renderKey = `${bucketKey}|${signature}`;
    if (form.dataset.renderKey === renderKey) return;
    if (normalized.length === 0) {
        form.innerHTML = '';
        form.dataset.renderKey = renderKey;
        form.dataset.locked = '0';
        form.dataset.lockBucket = bucketKey;
        return;
    }
    const answers = snapshotClarifyAnswers(form);
    form.innerHTML = '';
    normalized.forEach(q => {
        const div = document.createElement('div');
        div.style.marginBottom = '10px';
        const label = document.createElement('label');
        label.textContent = `${q.id}: ${q.question}`;
        div.appendChild(label);
        if (q.type === 'single') {
            q.options.forEach(opt => {
                const radio = document.createElement('input');
                radio.type = 'radio';
                radio.name = q.id;
                radio.value = opt;
                radio.checked = answers[q.id] === opt;
                div.appendChild(radio);
                const span = document.createElement('span');
                span.textContent = ` ${opt}`;
                div.appendChild(span);
                div.appendChild(document.createElement('br'));
            });
        } else if (q.type === 'multi') {
            q.options.forEach(opt => {
                const checkbox = document.createElement('input');
                checkbox.type = 'checkbox';
                checkbox.name = q.id;
                checkbox.value = opt;
                const prev = answers[q.id];
                checkbox.checked = Array.isArray(prev) ? prev.includes(opt) : prev === opt;
                div.appendChild(checkbox);
                const span = document.createElement('span');
                span.textContent = ` ${opt}`;
                div.appendChild(span);
                div.appendChild(document.createElement('br'));
            });
        } else {
            const input = document.createElement('input');
            input.type = 'text';
            input.name = q.id;
            input.value = String(answers[q.id] || '');
            div.appendChild(input);
        }
        form.appendChild(div);
    });
    form.dataset.renderKey = renderKey;
    form.dataset.locked = '0';
    form.dataset.lockBucket = bucketKey;
}

function shouldAllowClarifyInteraction() {
    return runSessionActive && !activeRunUnattendedMode;
}

function normalizeBucketUiStateForRender(state) {
    const normalized = { ...(state || defaultBucketUiState()) };
    let shouldPersist = false;
    if (!runSessionActive) {
        if (normalized.run_state !== 'idle') {
            normalized.run_state = 'idle';
            normalized.run_text = txt('run_idle', '');
            shouldPersist = true;
        }
        if ((normalized.status_class || '') !== '' || (normalized.status_text || '') !== txt('status_waiting', '')) {
            normalized.status_text = txt('status_waiting', '');
            normalized.status_class = '';
            shouldPersist = true;
        }
    }
    if (!shouldAllowClarifyInteraction() && Array.isArray(normalized.clarify_questions) && normalized.clarify_questions.length > 0) {
        normalized.clarify_questions = [];
        shouldPersist = true;
    }
    return { normalized, shouldPersist };
}

function resetBucketRuntimeUiState(bucket) {
    if (!bucket) return;
    const cur = readBucketUiState(bucket);
    if (
        (cur.run_state || 'idle') === 'idle'
        && (cur.run_text || txt('run_idle', '')) === txt('run_idle', '')
        && (cur.status_text || txt('status_waiting', '')) === txt('status_waiting', '')
        && (cur.status_class || '') === ''
        && (!Array.isArray(cur.clarify_questions) || cur.clarify_questions.length === 0)
    ) {
        return;
    }
    writeBucketUiState(bucket, {
        run_state: 'idle',
        run_text: txt('run_idle', ''),
        status_text: txt('status_waiting', ''),
        status_class: '',
        clarify_questions: [],
    });
}

function renderUiForViewBucket() {
    const bucket = viewLogBucket();
    const state = readBucketUiState(bucket);
    const { normalized, shouldPersist } = normalizeBucketUiStateForRender(state);
    if (shouldPersist) {
        writeBucketUiState(bucket, {
            status_text: normalized.status_text,
            status_class: normalized.status_class,
            run_state: normalized.run_state,
            run_text: normalized.run_text,
            clarify_questions: normalized.clarify_questions,
        });
    }
    const status = document.getElementById('status');
    status.textContent = normalized.status_text || txt('status_waiting', '');
    status.className = 'status-box ' + (normalized.status_class || '');
    const bar = document.getElementById('runbar');
    bar.dataset.state = normalized.run_state || 'idle';
    document.getElementById('runbar_text').textContent = fmt('runbar_line', '', { state: normalized.run_text || txt('run_idle', '') });
    document.getElementById('diagnostics').textContent = normalized.diagnostics_text || txt('diagnostics_none', '');
    renderDiffPanel(normalized.diff_text || '');
    const shouldShowClarify = shouldAllowClarifyInteraction() && Array.isArray(normalized.clarify_questions) && normalized.clarify_questions.length > 0;
    renderClarifyQuestions(shouldShowClarify ? (normalized.clarify_questions || []) : [], bucket, false);
}

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

function markBackendFailure(source, immediate) {
    if (immediate) {
        backendFailureCount = BACKEND_FAILURE_THRESHOLD;
    } else {
        backendFailureCount += 1;
    }
    if (backendFailureCount < BACKEND_FAILURE_THRESHOLD) return;
    if (backendOfflineNotified) return;
    backendOfflineNotified = true;
    setBackendOfflineState(true);
    const state = currentRunState();
    const runningLike = isRunStateActive(state);
    if (runningLike) {
        syncStoppedStateIfNeeded(fmt('sync_reason_backend_disconnected', '', { source }), true);
        applyInterruptedTerminalState({
            runTextKey: 'run_interrupted',
            statusKey: 'status_backend_disconnected',
            statusClass: 'status-danger',
        });
    } else {
        setStatus(txt('status_backend_offline', ''), 'status-danger');
    }
    alert(txt(runningLike ? 'alert_backend_disconnected' : 'alert_backend_offline', ''));
}

async function checkBackendHealth() {
    try {
        const resp = await fetch('/health', { cache: 'no-store' });
        if (!resp.ok) throw new Error(`health ${resp.status}`);
        markBackendAlive();
    } catch (_e) {
        markBackendFailure('health', true);
    }
}

function syncStoppedStateIfNeeded(reason, suppressStoppedAlert) {
    const state = currentRunState();
    if (!isRunStateActive(state)) return;
    finishRunSessionUi();
    setRunState('idle', txt('run_idle', ''));
    if (!suppressStoppedAlert) {
        showAlertOnce(txt('alert_task_stopped', ''));
    }
    if (reason) {
        appendLog(fmt('log_runtime_sync', '', { reason }));
    }
}

function isInterruptedMessage(msg) {
    const text = (msg || '').toLowerCase();
    const raw = txt(
        'interrupt_keywords',
        'stopped by user|interrupted|stop requested|cancelled|canceled|aborted|terminated|中断|停止|已停止|detenido|interrumpido|arrete|annule'
    );
    const keywords = raw
        .split('|')
        .map(x => x.trim().toLowerCase())
        .filter(Boolean);
    return keywords.some(k => text.includes(k));
}

function getFormData() {
    return {
        api_key: getApiKeyInputValue(),
        base_url: document.getElementById('base_url').value.trim(),
        model: document.getElementById('model').value.trim(),
        language: normalizeLanguageCode(sharedConfig.language || currentLanguage),
        auto_revert_profile: document.getElementById('auto_revert_profile').value.trim(),
        precheck_cmd: document.getElementById('precheck_cmd').value.trim(),
        release_gate_threshold: document.getElementById('release_gate_threshold').value.trim(),
        unattended_mode: !!document.getElementById('unattended_mode')?.checked,
        workspace: document.getElementById('workspace').value.trim(),
        goal: document.getElementById('goal').value.trim(),
        eval_cmd: document.getElementById('eval_cmd').value.trim(),
        success_regex: document.getElementById('success_regex').value.trim(),
        remote: document.getElementById('remote').value.trim(),
        remote_url: document.getElementById('remote_url').value.trim(),
        branch: document.getElementById('branch').value.trim(),
    };
}

function applyFormData(d) {
    if (!d) return;
    const fields = ['api_key', 'base_url', 'model', 'auto_revert_profile', 'precheck_cmd', 'release_gate_threshold', 'workspace', 'goal', 'eval_cmd', 'success_regex', 'remote', 'remote_url', 'branch'];
    fields.forEach(k => {
        if (typeof d[k] === 'string' && document.getElementById(k)) {
            document.getElementById(k).value = d[k];
        }
    });
    if (typeof d.unattended_mode === 'boolean') {
        const el = document.getElementById('unattended_mode');
        if (el) el.checked = d.unattended_mode;
    }
    if (typeof d.language === 'string' && d.language) {
        const code = normalizeLanguageCode(d.language);
        if (code) sharedConfig.language = code;
    }
    renderUnattendedState();
}

function applyStaticCopyToDom() {
    document.title = txt('app_title', document.title);
    const h1 = document.querySelector('.hero h1');
    if (h1) h1.textContent = txt('app_header', h1.textContent);
    const staticMap = [
        ['hero_sub', 'hero_sub'],
        ['backend_offline_banner', 'banner_backend_offline'],
        ['section_projects_title', 'section_projects_title'],
        ['section_projects_sub', 'section_projects_sub'],
        ['section_config_title', 'section_config_title'],
        ['section_config_sub', 'section_config_sub'],
        ['label_project_name', 'label_project_name'],
        ['label_unattended_mode', 'label_unattended_mode'],
        ['label_global_config', 'label_global_config'],
        ['label_auto_revert_profile', 'label_auto_revert_profile'],
        ['label_precheck_cmd', 'label_precheck_cmd'],
        ['label_release_gate_threshold', 'label_release_gate_threshold'],
        ['label_history_max_messages', 'label_history_max_messages'],
        ['label_history_max_chars', 'label_history_max_chars'],
        ['label_goal', 'label_goal'],
        ['label_eval_cmd', 'label_eval_cmd'],
        ['label_success_regex', 'label_success_regex'],
        ['section_git_title', 'section_git_title'],
        ['label_remote', 'label_remote'],
        ['label_remote_url', 'label_remote_url'],
        ['label_branch', 'label_branch'],
        ['hint_run_new_round', 'hint_run_new_round'],
        ['section_status_title', 'section_status_title'],
        ['unattended_state_text', 'unattended_state_default'],
        ['section_clarify_title', 'section_clarify_title'],
        ['section_log_title', 'section_log_title'],
        ['hint_log_exports', 'hint_log_exports'],
        ['section_diff_title', 'section_diff_title'],
        ['section_global_title', 'section_global_title'],
        ['label_api_key', 'label_api_key'],
        ['label_model', 'label_model'],
        ['label_base_url', 'label_base_url'],
        ['label_language', 'label_language'],
        ['label_global_auto_resume_attempts', 'label_global_auto_resume_attempts'],
        ['label_global_stop_after_minutes', 'label_global_stop_after_minutes'],
        ['label_global_log_max_chars', 'label_global_log_max_chars'],
        ['label_global_diff_max_chars', 'label_global_diff_max_chars'],
    ];
    staticMap.forEach(([id, key]) => {
        const el = document.getElementById(id);
        if (el) el.textContent = txt(key, '');
    });
    const map = [
        ['start_btn', 'btn_start'],
        ['resume_btn', 'btn_resume'],
        ['stop_btn', 'btn_stop'],
        ['go_running_project_btn', 'btn_go_running'],
        ['revert_btn', 'btn_revert'],
        ['push_btn', 'btn_push'],
        ['open_global_config_btn', 'btn_open_global'],
        ['save_global_config_btn', 'btn_save_global'],
        ['close_global_config_btn', 'btn_close'],
        ['project_new_btn', 'btn_project_new'],
        ['project_save_btn', 'btn_project_save'],
        ['project_load_btn', 'btn_project_load'],
        ['project_delete_btn', 'btn_project_delete'],
        ['clarify_submit_btn', 'btn_clarify_submit'],
        ['clear_log_btn', 'btn_clear_log'],
        ['export_log_btn', 'btn_export_log'],
        ['export_snapshot_btn', 'btn_export_snapshot'],
        ['export_report_btn', 'btn_export_report'],
    ];
    map.forEach(([id, key]) => {
        const el = document.getElementById(id);
        if (el) el.textContent = txt(key, '');
    });
    const logBtnTips = [
        ['export_log_btn', 'tip_export_log'],
        ['export_snapshot_btn', 'tip_export_snapshot'],
        ['export_report_btn', 'tip_export_report'],
    ];
    logBtnTips.forEach(([id, key]) => {
        const el = document.getElementById(id);
        if (el) el.title = txt(key, '');
    });
    const placeholderMap = [
        ['project_name', 'ph_project_name'],
        ['precheck_cmd', 'ph_precheck_cmd'],
        ['release_gate_threshold', 'ph_release_gate_threshold'],
        ['global_auto_resume_attempts', 'ph_global_auto_resume_attempts'],
        ['global_stop_after_minutes', 'ph_global_stop_after_minutes'],
        ['global_history_max_messages', 'ph_history_max_messages'],
        ['global_history_max_chars', 'ph_history_max_chars'],
        ['global_log_max_chars', 'ph_global_log_max_chars'],
        ['global_diff_max_chars', 'ph_global_diff_max_chars'],
        ['goal', 'ph_goal'],
        ['success_regex', 'ph_success_regex'],
        ['api_key', 'ph_api_key'],
        ['project_search', 'project_search_placeholder']
    ];
    placeholderMap.forEach(([id, key]) => {
        const el = document.getElementById(id);
        if (el) el.placeholder = txt(key, '');
    });
    const search = document.getElementById('project_search');
    if (search) search.placeholder = txt('project_search_placeholder', '');
    const summary = document.getElementById('project_summary_text');
    if (summary) {
        summary.textContent = fmt('project_summary_total', '', { total: loadProjects().length });
    }
    setProjectDraftState();
    renderProjectAccordion();
    applySidebarLayout();
    renderUnattendedState();
}

function openGlobalConfigModal() {
    closeSidebarOnNarrow();
    const modal = document.getElementById('global_config_modal');
    if (!modal) return;
    modal.style.display = 'flex';
    document.body.classList.add('modal-open');
}

function closeGlobalConfigModal() {
    const modal = document.getElementById('global_config_modal');
    if (!modal) return;
    modal.style.display = 'none';
    document.body.classList.remove('modal-open');
}

async function saveGlobalConfig() {
    updateGlobalOptionsFromInputs();
    maybeWarnLargeCharLimit(txt('label_global_config', ''));
    if (logMaxCharsSetting() > LARGE_CHAR_LIMIT_WARNING_THRESHOLD || diffMaxCharsSetting() > LARGE_CHAR_LIMIT_WARNING_THRESHOLD) {
        alert(fmt('warn_large_char_limit_alert', '', {
            threshold: LARGE_CHAR_LIMIT_WARNING_THRESHOLD,
            log_limit: logMaxCharsSetting(),
            diff_limit: diffMaxCharsSetting(),
        }));
    }
    const wantedLanguage = normalizeLanguageCode(document.getElementById('language_select')?.value || currentLanguage);
    if (wantedLanguage && wantedLanguage !== currentLanguage) {
        const switched = await switchLanguage(wantedLanguage, false);
        if (!switched) {
            setStatus(fmt('status_language_switch_failed', '', { code: wantedLanguage }), 'status-danger');
            return;
        }
    }
    const ok = await persistSharedConfigNow();
    if (!ok) {
        setStatus(txt('status_global_save_failed', ''), 'status-danger');
        return;
    }
    closeGlobalConfigModal();
    setStatus(txt('status_global_saved', ''), 'status-warn');
    renderCurrentLogView();
    renderUiForViewBucket();
}

function setAutoSaveState(text) {
    document.getElementById('auto_save_state').textContent = text;
}

function renderUnattendedState() {
    const el = document.getElementById('unattended_state_text');
    if (!el) return;
    const configuredUnattendedMode = !!document.getElementById('unattended_mode')?.checked;
    const unattendedMode = runSessionActive ? activeRunUnattendedMode : configuredUnattendedMode;
    const mins = stopAfterMinutesSetting();
    let stopText = txt('unattended_stop_disabled', '');
    if (mins > 0) {
        if (!runSessionActive) {
            stopText = fmt('unattended_stop_left_minutes', '', { minutes: mins });
        } else if (manualRunStopExpired) {
            stopText = txt('unattended_stop_expired', '');
        } else {
            const remaining = manualRunStopDeadlineMs > 0
                ? Math.max(0, Math.ceil((manualRunStopDeadlineMs - Date.now()) / 60000))
                : mins;
            stopText = fmt('unattended_stop_left_minutes', '', { minutes: remaining });
        }
    }
    const resumeLeft = runSessionActive
        ? Math.max(0, unattendedAutoResumeRemaining)
        : Math.max(0, autoResumeAttemptsSetting());
    el.textContent = fmt('unattended_state_line', '', {
        mode: unattendedMode ? txt('unattended_mode_on', '') : txt('unattended_mode_off', ''),
        resume_left: String(resumeLeft),
        stop: stopText,
    });
}

function clearUnattendedAutoResumeTimer() {
    if (!unattendedAutoResumeTimer) return;
    clearTimeout(unattendedAutoResumeTimer);
    unattendedAutoResumeTimer = null;
}

function clearManualRunStopTimer() {
    if (!manualRunStopTimer) return;
    clearTimeout(manualRunStopTimer);
    manualRunStopTimer = null;
}

function stopAfterMinutesSetting() {
    return parseBoundedInt(globalOptions.stop_after_minutes, 0, 24 * 60, 0);
}

function armManualRunStopTimer() {
    clearManualRunStopTimer();
    manualRunStopExpired = false;
    const mins = stopAfterMinutesSetting();
    if (mins <= 0) {
        manualRunStopDeadlineMs = 0;
        renderUnattendedState();
        return;
    }
    manualRunStopDeadlineMs = Date.now() + mins * 60 * 1000;
    renderUnattendedState();
    manualRunStopTimer = setTimeout(() => {
        manualRunStopExpired = true;
        appendLog(fmt('log_stop_timer_expired', '', { minutes: mins }));
        setStatus(fmt('status_stop_timer_wait_round', '', { minutes: mins }), 'status-warn');
        renderUnattendedState();
    }, mins * 60 * 1000);
}

async function triggerStopByTimeout() {
    if (!runSessionActive || stopRequested) return;
    manualRunStopExpired = false;
    appendLog(txt('log_stop_timer_trigger_stop', ''));
    renderUnattendedState();
    await stopSession();
}

function maybeStopOnRoundBoundary(line) {
    if (!manualRunStopExpired) return;
    if (!line.startsWith('[Loop] Iteration')) return;
    triggerStopByTimeout();
}

function autoResumeAttemptsSetting() {
    return parseBoundedInt(globalOptions.auto_resume_attempts, 0, 20, 0);
}

function scheduleUnattendedAutoResume() {
    if (!activeRunUnattendedMode || unattendedAutoResumeRemaining <= 0) return false;
    clearUnattendedAutoResumeTimer();
    const nextTry = unattendedAutoResumeRemaining;
    unattendedAutoResumeRemaining -= 1;
    renderUnattendedState();
    appendLog(fmt('log_unattended_auto_resume_scheduled', '', {
        try: nextTry,
        left: unattendedAutoResumeRemaining,
    }));
    unattendedAutoResumeTimer = setTimeout(() => {
        resumeSession({ auto: true });
    }, 1200);
    return true;
}

function resetStopTimerState() {
    clearManualRunStopTimer();
    manualRunStopDeadlineMs = 0;
    manualRunStopExpired = false;
}

function resetUnattendedRunState() {
    activeRunUnattendedMode = false;
    unattendedAutoResumeRemaining = 0;
    clearUnattendedAutoResumeTimer();
    resetStopTimerState();
    renderUnattendedState();
}

function resetRunSessionUiState() {
    activeRunLogBucket = '';
    activeRunProjectLabel = '';
    runSessionActive = false;
    setRunningProjectIndicator(txt('running_project_none', ''));
    setProjectControlsDisabled(false);
    applyReadOnlyMode();
}

function showAlertOnce(message) {
    if (completionAlertShown) return false;
    completionAlertShown = true;
    alert(message);
    return true;
}

function setRunStateAndStatus(runState, runTextKey, statusKey, statusClass, message) {
    setRunState(runState, txt(runTextKey, ''));
    if (typeof message === 'string') {
        setStatus(fmt(statusKey, '', { message }), statusClass);
    } else {
        setStatus(txt(statusKey, ''), statusClass);
    }
}

function finishRunSessionUi() {
    clearStopAckTimer();
    stopRequested = false;
    resetRunSessionUiState();
    setRunActionButtons(false);
}

function markRunSessionStarted(label, resetFlags) {
    setRunningProjectIndicator(label);
    setProjectControlsDisabled(true);
    runSessionActive = true;
    applyReadOnlyMode();
    setRunActionButtons(true);
    document.getElementById('revert_btn').disabled = false;
    if (resetFlags) {
        stopRequested = false;
        completionAlertShown = false;
    }
}

const DONE_OUTCOME_CONFIG = {
    success: {
        runState: 'success',
        runTextKey: 'run_success',
        statusKey: 'status_done_success',
        statusClass: 'status-ok',
        alertKey: 'alert_done_success',
        resetUnattended: true,
        allowAutoResume: false,
    },
    interrupted: {
        runState: 'interrupted',
        runTextKey: 'run_interrupted',
        statusKey: 'status_done_interrupted',
        statusClass: 'status-warn',
        alertKey: 'alert_done_interrupted',
        resetUnattended: true,
        allowAutoResume: false,
    },
    failed: {
        runState: 'failed',
        runTextKey: 'run_failed',
        statusKey: 'status_done_failed',
        statusClass: 'status-danger',
        alertKey: 'alert_done_failed',
        resetUnattended: false,
        allowAutoResume: true,
    },
};

function applyDoneOutcome(outcome, message) {
    const cfg = DONE_OUTCOME_CONFIG[outcome] || DONE_OUTCOME_CONFIG.failed;
    setRunStateAndStatus(cfg.runState, cfg.runTextKey, cfg.statusKey, cfg.statusClass, message);
    if (cfg.resetUnattended) {
        resetUnattendedRunState();
    }
    let autoResumed = false;
    if (cfg.allowAutoResume) {
        autoResumed = scheduleUnattendedAutoResume();
    }
    if (!autoResumed) {
        showAlertOnce(fmt(cfg.alertKey, '', { message }));
    }
}

function applyInterruptedTerminalState(options) {
    const cfg = options || {};
    setRunStateAndStatus(
        'interrupted',
        cfg.runTextKey || 'run_interrupted',
        cfg.statusKey || 'status_done_interrupted',
        cfg.statusClass || 'status-warn',
        cfg.message
    );
    if (cfg.alertKey) {
        showAlertOnce(
            typeof cfg.message === 'string'
                ? fmt(cfg.alertKey, '', { message: cfg.message })
                : txt(cfg.alertKey, '')
        );
    }
}

const STOP_ACCEPTED_STATUS_CONFIG = {
    accepted: { key: 'status_stop_waiting_ack', className: 'status-warn' },
    channel_unavailable: { key: 'status_stop_backend_done', className: 'status-warn' },
};

function applyStopAcceptedStatus(ack) {
    const cfg = ack.includes('channel unavailable')
        ? STOP_ACCEPTED_STATUS_CONFIG.channel_unavailable
        : STOP_ACCEPTED_STATUS_CONFIG.accepted;
    setRunState('stopping', txt('run_stopping', ''));
    setStatus(txt(cfg.key, ''), cfg.className);
    setRunActionButtons(false);
}

