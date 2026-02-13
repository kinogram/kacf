let lastEventId = 0;
let polling = false;
let saveTimer = null;
let projectAutoSaveTimer = null;
let projectAutoSaveInFlight = false;
let stopRequested = false;
let stopAckTimer = null;
let eventSource = null;
let sseErrorCount = 0;
let sseConnectStartedAt = 0;
let pollingStarted = false;
let pollTimer = null;
let lastSseMessageAt = 0;
let completionAlertShown = false;
let lastRealtimeFallbackReason = '';
let lastRealtimeFallbackAt = 0;
let backendFailureCount = 0;
let backendOfflineNotified = false;
let backendOffline = false;
let backendReconnectReloading = false;
const POLL_MS_ACTIVE = 1000;
const POLL_MS_IDLE = 3000;
const UI_STATE_SYNC_MS = 3000;
const SSE_STALE_MS = 8000;
const FALLBACK_LOG_DEDUP_MS = 30000;
const BACKEND_FAILURE_THRESHOLD = 3;
const SSE_ERROR_LIMIT = 6;
const SSE_CONNECT_GRACE_MS = 10000;
const DEFAULT_LOG_MAX_CHARS = 50000;
const DEFAULT_DIFF_MAX_CHARS = 50000;
const LARGE_CHAR_LIMIT_WARNING_THRESHOLD = 50000;
const GLOBAL_OPTION_DEFAULTS = {
    auto_resume_attempts: '',
    stop_after_minutes: '',
    history_max_messages: '40',
    history_max_chars: '70000',
    log_max_chars: String(DEFAULT_LOG_MAX_CHARS),
    diff_max_chars: String(DEFAULT_DIFF_MAX_CHARS),
};
const GLOBAL_OPTION_INPUT_ID_BY_KEY = {
    auto_resume_attempts: 'global_auto_resume_attempts',
    stop_after_minutes: 'global_stop_after_minutes',
    history_max_messages: 'global_history_max_messages',
    history_max_chars: 'global_history_max_chars',
    log_max_chars: 'global_log_max_chars',
    diff_max_chars: 'global_diff_max_chars',
};
let activeRunLogBucket = '';
let activeRunProjectLabel = '';
let runSessionActive = false;
let cachedProjects = [];
let cacheProjectLogs = {};
let cacheProjectUiState = {};
let logRenderStateByBucket = {};
let logBucketTouchedAt = {};
let uiStateBucketTouchedAt = {};
let uiCacheSaveTimer = null;
let uiCacheLastHeavySyncMs = 0;
const LOG_BUCKET_MAX_COUNT = 40;
const UI_STATE_BUCKET_MAX_COUNT = 60;
const LOG_TOTAL_MAX_CHARS = 400000;
const WORKSPACE_ROOT = './autocoding_data/workspaces';
let projectNameManualOverride = false;
let lastAutoProjectName = '';
let availableLanguages = [];
let currentLanguage = '';
let projectSearchKeyword = '';
let projectDraftDirty = false;
let sidebarCollapsed = false;
let sidebarMobileOpen = false;
let globalOptions = { ...GLOBAL_OPTION_DEFAULTS };
let unattendedAutoResumeRemaining = 0;
let unattendedAutoResumeTimer = null;
let activeRunUnattendedMode = false;
let manualRunStopTimer = null;
let manualRunStopDeadlineMs = 0;
let manualRunStopExpired = false;
let sharedConfig = {
    api_key: '',
    base_url: 'https://api.deepseek.com',
    model: 'deepseek-reasoner',
    language: '',
};
let COPY = {};
const BACKEND_COMM_BUTTON_IDS = [
    'start_btn',
    'resume_btn',
    'stop_btn',
    'revert_btn',
    'push_btn',
    'clarify_submit_btn',
    'project_new_btn',
    'project_save_btn',
    'project_load_btn',
    'project_delete_btn',
    'open_global_config_btn',
    'save_global_config_btn',
];
const PROJECT_CONTROL_IDS = [
    'project_name',
    'project_new_btn',
    'project_save_btn',
    'project_load_btn',
    'project_delete_btn',
];
const CONFIG_EDIT_IDS = [
    'api_key', 'model', 'base_url', 'workspace', 'auto_revert_profile',
    'precheck_cmd', 'release_gate_threshold',
    'unattended_mode',
    'goal', 'eval_cmd', 'success_regex', 'remote', 'remote_url', 'branch',
    'open_global_config_btn', 'language_select',
];

function normalizeLanguageCode(raw) {
    const v = (raw || '').trim();
    if (!v) return '';
    if (!/^[A-Za-z0-9_-]{1,32}$/.test(v)) return '';
    return v;
}

function parseBoundedInt(raw, min, max, fallback) {
    const n = Number.parseInt(String(raw || '').trim(), 10);
    if (!Number.isFinite(n)) return fallback;
    if (n < min) return min;
    if (n > max) return max;
    return n;
}

function normalizeGlobalOptions(raw) {
    const out = { ...GLOBAL_OPTION_DEFAULTS };
    if (!raw || typeof raw !== 'object') return out;
    Object.keys(GLOBAL_OPTION_DEFAULTS).forEach((key) => {
        out[key] = String(raw[key] ?? GLOBAL_OPTION_DEFAULTS[key]).trim();
    });
    return out;
}

function logMaxCharsSetting() {
    return parseBoundedInt(globalOptions.log_max_chars, 1, 2_000_000, DEFAULT_LOG_MAX_CHARS);
}

function diffMaxCharsSetting() {
    return parseBoundedInt(globalOptions.diff_max_chars, 1, 2_000_000, DEFAULT_DIFF_MAX_CHARS);
}

function maybeWarnLargeCharLimit(source) {
    const logLimit = logMaxCharsSetting();
    const diffLimit = diffMaxCharsSetting();
    if (logLimit <= LARGE_CHAR_LIMIT_WARNING_THRESHOLD && diffLimit <= LARGE_CHAR_LIMIT_WARNING_THRESHOLD) {
        return;
    }
    setStatus(fmt('status_large_char_limit_warning', '', {
        source: source || txt('label_global_config', ''),
        threshold: LARGE_CHAR_LIMIT_WARNING_THRESHOLD,
        log_limit: logLimit,
        diff_limit: diffLimit,
    }), 'status-warn');
}

function truncateTailChars(raw, maxChars) {
    const text = String(raw || '');
    if (maxChars <= 0) return '';
    if (text.length <= maxChars) return text;
    return text.slice(text.length - maxChars);
}

function sanitizeDiffText(raw) {
    return truncateTailChars(raw, diffMaxCharsSetting());
}

function sanitizeLogContent(raw) {
    const tail = truncateTailChars(raw, logMaxCharsSetting());
    if (!tail) return '';
    return tail.endsWith('\n') ? tail : `${tail}\n`;
}

function sanitizeProjectLogsMap(rawMap) {
    const out = {};
    if (!rawMap || typeof rawMap !== 'object') return out;
    Object.keys(rawMap).forEach(k => {
        if (typeof rawMap[k] !== 'string') return;
        out[String(k)] = sanitizeLogContent(rawMap[k]);
    });
    return out;
}

function sanitizeProjectUiStateMap(rawMap) {
    const out = {};
    if (!rawMap || typeof rawMap !== 'object') return out;
    Object.keys(rawMap).forEach(k => {
        const entry = rawMap[k];
        if (!entry || typeof entry !== 'object') return;
        const normalized = { ...entry };
        if (typeof normalized.diff_text === 'string') {
            normalized.diff_text = sanitizeDiffText(normalized.diff_text);
        }
        out[String(k)] = normalized;
    });
    return out;
}

function touchBucket(metaMap, bucket) {
    metaMap[String(bucket || '')] = Date.now();
}

function pruneLogBuckets() {
    const entries = Object.entries(cacheProjectLogs || {});
    if (!entries.length) return;
    const score = (k) => logBucketTouchedAt[k] || 0;
    const isProject = (k) => String(k).startsWith('project:');
    let keys = entries.map(([k]) => k);
    keys.sort((a, b) => score(a) - score(b));
    while (keys.length > LOG_BUCKET_MAX_COUNT) {
        const idx = keys.findIndex(k => !isProject(k));
        const victim = idx >= 0 ? keys[idx] : keys[0];
        delete cacheProjectLogs[victim];
        delete logBucketTouchedAt[victim];
        delete cacheProjectUiState[victim];
        delete uiStateBucketTouchedAt[victim];
        delete logRenderStateByBucket[victim];
        keys = keys.filter(k => k !== victim);
    }
    let total = Object.values(cacheProjectLogs).reduce((acc, v) => acc + String(v || '').length, 0);
    if (total <= LOG_TOTAL_MAX_CHARS) return;
    const sorted = Object.keys(cacheProjectLogs).sort((a, b) => score(a) - score(b));
    for (const k of sorted) {
        if (total <= LOG_TOTAL_MAX_CHARS) break;
        if (isProject(k) && k === currentLogBucket()) continue;
        total -= String(cacheProjectLogs[k] || '').length;
        delete cacheProjectLogs[k];
        delete logBucketTouchedAt[k];
        delete cacheProjectUiState[k];
        delete uiStateBucketTouchedAt[k];
        delete logRenderStateByBucket[k];
    }
}

function pruneUiStateBuckets() {
    const keys = Object.keys(cacheProjectUiState || {});
    if (keys.length <= UI_STATE_BUCKET_MAX_COUNT) return;
    keys.sort((a, b) => (uiStateBucketTouchedAt[a] || 0) - (uiStateBucketTouchedAt[b] || 0));
    while (keys.length > UI_STATE_BUCKET_MAX_COUNT) {
        const victim = keys.shift();
        if (!victim) break;
        delete cacheProjectUiState[victim];
        delete uiStateBucketTouchedAt[victim];
    }
}

function languageLabel(code) {
    const norm = normalizeLanguageCode(code);
    if (!norm) return txt('language_unknown', '');
    if (norm === 'zh-CN') return txt('language_name_zh_cn', '');
    if (norm === 'en') return txt('language_name_en', '');
    return norm;
}

function isNarrowViewport() {
    return window.matchMedia('(max-width: 1023px)').matches;
}

function applySidebarLayout() {
    const root = document.body;
    const narrow = isNarrowViewport();
    root.classList.toggle('sidebar-collapsed', !narrow && sidebarCollapsed);
    root.classList.toggle('sidebar-open', narrow && sidebarMobileOpen);
    const t = document.getElementById('sidebar_toggle_btn');
    if (t) {
        t.textContent = narrow
            ? txt('btn_sidebar_close', '')
            : (sidebarCollapsed ? txt('btn_sidebar_expand', '') : txt('btn_sidebar_collapse', ''));
        t.title = t.textContent;
    }
    const m = document.getElementById('sidebar_mobile_btn');
    if (m) m.textContent = txt('btn_sidebar_open', '');
}

function setSidebarCollapsed(next) {
    sidebarCollapsed = !!next;
    try { localStorage.setItem('kacf_sidebar_collapsed', sidebarCollapsed ? '1' : '0'); } catch (_e) {}
    applySidebarLayout();
}

function setSidebarMobileOpen(next) {
    sidebarMobileOpen = !!next;
    applySidebarLayout();
}

function closeSidebarOnNarrow() {
    if (!isNarrowViewport()) return;
    setSidebarMobileOpen(false);
}

function choosePreferredLanguage(langs, savedLanguage) {
    const items = Array.isArray(langs) ? langs.map(normalizeLanguageCode).filter(Boolean) : [];
    if (!items.length) return '';
    if (items.length === 1) return items[0];
    const saved = normalizeLanguageCode(savedLanguage);
    if (saved && items.includes(saved)) return saved;
    const navList = Array.isArray(navigator.languages) && navigator.languages.length
        ? navigator.languages
        : [navigator.language || ''];
    const isZh = navList.some(x => (x || '').toLowerCase().startsWith('zh'));
    if (isZh) {
        if (items.includes('zh-CN')) return 'zh-CN';
        const anyZh = items.find(x => x.toLowerCase().startsWith('zh'));
        if (anyZh) return anyZh;
    }
    if (items.includes('en')) return 'en';
    return items[0];
}

async function fetchLanguagesFromServer() {
    const resp = await fetch('/assets/languages/list');
    if (!resp.ok) throw new Error(`language list failed: ${resp.status}`);
    const data = await resp.json();
    const languages = Array.isArray(data.languages) ? data.languages : [];
    return languages.map(normalizeLanguageCode).filter(Boolean);
}

async function loadCopyForLanguage(code) {
    const lang = normalizeLanguageCode(code);
    if (!lang) return false;
    try {
        const resp = await fetch(`/assets/languages/${encodeURIComponent(lang)}.json`);
        if (!resp.ok) return false;
        const data = await resp.json();
        if (!data || typeof data !== 'object') return false;
        COPY = data;
        currentLanguage = lang;
        document.documentElement.lang = lang;
        return true;
    } catch (_e) {
        return false;
    }
}

function renderLanguageOptions() {
    const select = document.getElementById('language_select');
    if (!select) return;
    const items = Array.isArray(availableLanguages) ? availableLanguages : [];
    select.innerHTML = '';
    items.forEach(code => {
        const opt = document.createElement('option');
        opt.value = code;
        opt.textContent = languageLabel(code);
        select.appendChild(opt);
    });
    const selected = normalizeLanguageCode(sharedConfig.language) || currentLanguage || items[0] || '';
    if (selected) {
        select.value = selected;
    }
}

async function switchLanguage(code, persist) {
    const lang = normalizeLanguageCode(code);
    if (!lang) return false;
    const ok = await loadCopyForLanguage(lang);
    if (!ok) return false;
    sharedConfig.language = lang;
    renderLanguageOptions();
    applyStaticCopyToDom();
    if (persist) scheduleUiCacheSave();
    return true;
}

async function initLanguagePack() {
    availableLanguages = await fetchLanguagesFromServer();
    if (!availableLanguages.length) {
        throw new Error('no language packs from backend');
    }
    const picked = choosePreferredLanguage(availableLanguages, sharedConfig.language);
    const loaded = await switchLanguage(picked, false);
    if (!loaded) {
        throw new Error(`failed to load language pack: ${picked}`);
    }
}

function txt(key, fallback) {
    return COPY[key] || fallback || key;
}

function fmt(key, fallback, vars) {
    let out = txt(key, fallback);
    const map = vars || {};
    Object.keys(map).forEach(k => {
        out = out.replaceAll(`{${k}}`, String(map[k]));
    });
    return out;
}

function inferProjectNameFromGoal(goal) {
    let text = (goal || '').replace(/\s+/g, ' ').trim();
    if (!text) return '';
    text = text
        .replace(/^(做一个|开发一个|实现一个|创建一个|请做一个|请实现一个|帮我做一个)\s*/i, '')
        .replace(/^(build|create|implement|develop)\s+(a|an)?\s*/i, '');
    const cut = text.search(/[。！？.!?]/);
    if (cut > 0) text = text.slice(0, cut);
    text = text.trim();
    if (!text) return '';
    const maxLen = 26;
    if (text.length > maxLen) text = text.slice(0, maxLen).trim();
    return text;
}

function autoFillProjectNameFromGoal(force) {
    const el = document.getElementById('project_name');
    if (!el) return;
    const current = el.value.trim();
    const canAuto = force || !projectNameManualOverride || !current || current === lastAutoProjectName;
    if (!canAuto) return;
    const next = inferProjectNameFromGoal(document.getElementById('goal')?.value || '');
    if (!next) return;
    el.value = next;
    lastAutoProjectName = next;
    projectNameManualOverride = false;
}

function getApiKeyInputValue() {
    const el = document.getElementById('api_key');
    if (!el) return '';
    return el.value.trim();
}

function renderApiKeyInput() {
    const el = document.getElementById('api_key');
    if (!el) return;
    el.type = 'text';
    el.readOnly = false;
    el.value = (sharedConfig.api_key || '').trim();
}

async function suggestProjectSlug(projectName, goal) {
    try {
        const resp = await fetch('/projects/suggest_slug', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                project_name: projectName || '',
                goal: goal || '',
                api_key: sharedConfig.api_key || '',
                base_url: sharedConfig.base_url || 'https://api.deepseek.com',
                model: sharedConfig.model || 'deepseek-reasoner',
            }),
        });
        if (!resp.ok) return `project-${Date.now()}`;
        const data = await resp.json();
        const slug = (data.slug || '').trim();
        if (!slug) return `project-${Date.now()}`;
        return slug;
    } catch (_e) {
        return `project-${Date.now()}`;
    }
}

function workspaceLeaf(path) {
    const p = (path || '').trim();
    if (!p) return '-';
    const segs = p.split('/');
    return segs[segs.length - 1] || p;
}

function ensureUniqueWorkspace(basePath, items, selfId) {
    const used = new Set(
        (items || [])
            .filter(x => x && x.id !== selfId)
            .map(x => (x.workspace || '').trim())
            .filter(Boolean)
    );
    if (!used.has(basePath)) return basePath;
    for (let i = 2; i < 1000; i++) {
        const next = `${basePath}-${i}`;
        if (!used.has(next)) return next;
    }
    return `${basePath}-${Date.now()}`;
}

function defaultFormData() {
    return {
        api_key: sharedConfig.api_key || '',
        base_url: sharedConfig.base_url || 'https://api.deepseek.com',
        model: sharedConfig.model || 'deepseek-reasoner',
        auto_revert_profile: 'balanced',
        precheck_cmd: '',
        release_gate_threshold: '',
        unattended_mode: false,
        workspace: '',
        goal: '',
        eval_cmd: 'bash scripts/run_tests.sh',
        success_regex: '',
        remote: 'origin',
        remote_url: '',
        branch: 'main',
    };
}

function updateSharedConfigFromInputs() {
    sharedConfig = {
        api_key: getApiKeyInputValue(),
        base_url: document.getElementById('base_url').value.trim(),
        model: document.getElementById('model').value.trim(),
        language: normalizeLanguageCode(document.getElementById('language_select')?.value || currentLanguage),
    };
}

function applySharedConfigToInputs() {
    renderApiKeyInput();
    document.getElementById('base_url').value = sharedConfig.base_url || 'https://api.deepseek.com';
    document.getElementById('model').value = sharedConfig.model || 'deepseek-reasoner';
    renderLanguageOptions();
}

function nowText() {
    return new Date().toLocaleString();
}

function setRunningProjectIndicator(label) {
    const el = document.getElementById('running_project_text');
    if (!el) return;
    el.textContent = fmt('running_project_line', '', { name: label || txt('running_project_none', '') });
    renderProjectAccordion();
}

function setRunActionButtons(running) {
    const startBtn = document.getElementById('start_btn');
    const stopBtn = document.getElementById('stop_btn');
    if (startBtn) startBtn.disabled = !!running;
    if (stopBtn) stopBtn.disabled = !running;
}

function setElementsDisabled(ids, disabled) {
    ids.forEach(id => {
        const el = document.getElementById(id);
        if (!el) return;
        el.disabled = !!disabled;
    });
}

function setProjectControlsDisabled(disabled) {
    setElementsDisabled(PROJECT_CONTROL_IDS, disabled);
}

function setConfigInputsDisabled(disabled) {
    setElementsDisabled(CONFIG_EDIT_IDS, disabled);
}

function applyBackendOfflineMode() {
    BACKEND_COMM_BUTTON_IDS.forEach(id => {
        const el = document.getElementById(id);
        if (!el) return;
        if (backendOffline) {
            if (el.dataset.backendOfflineLocked !== '1') {
                el.dataset.backendOfflinePrevDisabled = el.disabled ? '1' : '0';
                el.dataset.backendOfflineLocked = '1';
            }
            el.style.display = 'none';
            el.disabled = true;
            return;
        }
        el.style.display = '';
        if (el.dataset.backendOfflineLocked === '1') {
            el.disabled = el.dataset.backendOfflinePrevDisabled === '1';
            delete el.dataset.backendOfflinePrevDisabled;
            delete el.dataset.backendOfflineLocked;
        }
    });
}

function setOfflineInputLock(locked) {
    const fields = document.querySelectorAll('input, textarea, select');
    fields.forEach(el => {
        if (el.id === 'workspace') return;
        if (locked) {
            if (el.dataset.offlineLocked !== '1') {
                el.dataset.offlinePrevDisabled = el.disabled ? '1' : '0';
                if ('readOnly' in el) {
                    el.dataset.offlinePrevReadonly = el.readOnly ? '1' : '0';
                }
                el.dataset.offlineLocked = '1';
            }
            if ('readOnly' in el) el.readOnly = true;
            el.disabled = true;
            return;
        }
        if (el.dataset.offlineLocked === '1') {
            if ('readOnly' in el) {
                el.readOnly = el.dataset.offlinePrevReadonly === '1';
            }
            el.disabled = el.dataset.offlinePrevDisabled === '1';
            delete el.dataset.offlineLocked;
            delete el.dataset.offlinePrevDisabled;
            delete el.dataset.offlinePrevReadonly;
        }
    });
}

function isReadOnlyView() {
    if (!runSessionActive) return false;
    if (!activeRunLogBucket) return false;
    return viewLogBucket() !== activeRunLogBucket;
}

function applyReadOnlyMode() {
    setOfflineInputLock(false);
    const viewReadOnly = isReadOnlyView();
    const readOnly = backendOffline || viewReadOnly;
    const lockConfigInputs = backendOffline || runSessionActive || viewReadOnly;
    const lockProjectControls = backendOffline || runSessionActive;
    const hint = document.getElementById('readonly_mode_text');
    if (hint) {
        if (backendOffline) {
            hint.textContent = txt('readonly_backend_offline', '');
        } else {
            hint.textContent = readOnly
                ? txt('readonly_on', '')
                : txt('readonly_off', '');
        }
    }
    setConfigInputsDisabled(lockConfigInputs);
    document.getElementById('start_btn').disabled = readOnly || runSessionActive;
    document.getElementById('resume_btn').disabled = readOnly || runSessionActive;
    document.getElementById('stop_btn').disabled = readOnly || !runSessionActive;
    document.getElementById('revert_btn').disabled = readOnly || !runSessionActive;
    document.getElementById('push_btn').disabled = readOnly;
    document.getElementById('clarify_submit_btn').disabled = readOnly;
    setProjectControlsDisabled(lockProjectControls);
    applyBackendOfflineMode();
    setOfflineInputLock(backendOffline);
    const offlineBanner = document.getElementById('backend_offline_banner');
    if (offlineBanner) {
        offlineBanner.textContent = txt(
            'banner_backend_offline',
            ''
        );
        offlineBanner.style.display = backendOffline ? 'block' : 'none';
    }
    updateGoRunningProjectButton();
}

function updateGoRunningProjectButton() {
    const btn = document.getElementById('go_running_project_btn');
    if (!btn) return;
    if (!runSessionActive || !activeRunLogBucket) {
        btn.disabled = true;
        btn.title = txt('status_no_running_project', '');
        return;
    }
    if (!activeRunLogBucket.startsWith('project:')) {
        btn.disabled = true;
        btn.title = txt('status_running_not_saved_project', '');
        return;
    }
    const id = activeRunLogBucket.slice('project:'.length);
    const exists = loadProjects().some(x => x.id === id);
    btn.disabled = !exists;
    btn.title = exists ? txt('go_running_title_ok', '') : txt('go_running_title_missing', '');
}

function goToRunningProjectView() {
    if (!runSessionActive || !activeRunLogBucket) {
        setStatus(txt('status_no_running_project', ''), 'status-danger');
        return;
    }
    if (!activeRunLogBucket.startsWith('project:')) {
        setStatus(txt('status_running_not_saved_project', ''), 'status-danger');
        return;
    }
    const id = activeRunLogBucket.slice('project:'.length);
    const sel = document.getElementById('project_selector');
    if (!sel) return;
    sel.value = id;
    syncProjectNameFromSelection();
    setStatus(fmt('status_go_running_ok', '', { name: activeRunProjectLabel || id }), 'status-warn');
}

function currentProjectLabel() {
    const sel = document.getElementById('project_selector');
    const selectedId = sel?.value || '';
    if (selectedId) {
        const item = loadProjects().find(x => x.id === selectedId);
        if (item && (item.name || item.workspace)) {
            return item.name || item.workspace;
        }
    }
    const body = getFormData();
    const ws = (body.workspace || '').trim();
    const goal = (body.goal || '').trim();
    if (ws) return ws;
    if (goal) return goal.slice(0, 24);
    return txt('project_unnamed', '');
}

function adHocLogBucketFromForm() {
    const body = getFormData();
    const ws = (body.workspace || '').trim() || './workspace';
    const goal = (body.goal || '').trim() || '(empty-goal)';
    return `adhoc:${ws}::${goal}`;
}

function viewLogBucket() {
    const selectedProjectId = document.getElementById('project_selector')?.value || '';
    if (selectedProjectId) return `project:${selectedProjectId}`;
    return adHocLogBucketFromForm();
}

function currentLogBucket() {
    return activeRunLogBucket || viewLogBucket();
}

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

