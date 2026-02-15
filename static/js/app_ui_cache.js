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
    return merged;
}

function writeBucketUiState(bucket, patch) {
    const all = loadProjectUiStateMap();
    const next = { ...readBucketUiState(bucket), ...(patch || {}) };
    all[bucket] = next;
    touchBucket(uiStateBucketTouchedAt, bucket);
    pruneUiStateBuckets();
    saveProjectUiStateMap(all);
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
    const runText = fmt('runbar_line', '', { state: normalized.run_text || txt('run_idle', '') });
    const runEl = document.getElementById('runbar_text');
    if (typeof setTopbarHintText === 'function') {
        setTopbarHintText(runEl, runText);
    } else if (runEl) {
        runEl.textContent = runText;
    }
    document.getElementById('diagnostics').textContent = normalized.diagnostics_text || txt('diagnostics_none', '');
    const shouldShowClarify = shouldAllowClarifyInteraction() && Array.isArray(normalized.clarify_questions) && normalized.clarify_questions.length > 0;
    renderClarifyQuestions(shouldShowClarify ? (normalized.clarify_questions || []) : [], bucket, false);
}

window.KACF = window.KACF || {};
window.KACF.uiCache = {
    fetchUiCacheFromServer,
    scheduleUiCacheSave,
    persistSharedConfigNow,
    updateGlobalOptionsFromInputs,
    applyGlobalOptionsToInputs,
    bindGlobalOptionInput,
    loadProjectUiStateMap,
    saveProjectUiStateMap,
    defaultBucketUiState,
    readBucketUiState,
    writeBucketUiState,
    normalizeClarifyQuestions,
    snapshotClarifyAnswers,
    renderClarifyQuestions,
    shouldAllowClarifyInteraction,
    normalizeBucketUiStateForRender,
    resetBucketRuntimeUiState,
    renderUiForViewBucket,
};
