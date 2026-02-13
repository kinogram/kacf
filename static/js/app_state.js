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

