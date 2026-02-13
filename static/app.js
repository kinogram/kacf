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
let activeRunLogBucket = '';
let activeRunProjectLabel = '';
let runSessionActive = false;
let cachedProjects = [];
let cacheProjectLogs = {};
let cacheProjectUiState = {};
let uiCacheSaveTimer = null;
const WORKSPACE_ROOT = './autocoding_data/workspaces';
let projectNameManualOverride = false;
let lastAutoProjectName = '';
let availableLanguages = [];
let currentLanguage = '';
let projectSearchKeyword = '';
let projectDraftDirty = false;
let sidebarCollapsed = false;
let sidebarMobileOpen = false;
let apiKeyEditMode = false;
let sharedConfig = {
    api_key: '',
    base_url: 'https://api.deepseek.com',
    model: 'deepseek-reasoner',
    language: '',
    encrypt_api_key: false,
    mask_api_key: false,
    api_key_is_masked: false,
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
    'api_key_edit_btn',
    'preset_webapp_btn',
    'preset_cli_btn',
    'preset_desktop_btn',
    'reset_form_btn',
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
    'precheck_cmd', 'history_max_messages', 'history_max_chars', 'release_gate_threshold',
    'goal', 'eval_cmd', 'success_regex', 'remote', 'remote_url', 'branch',
    'preset_webapp_btn', 'preset_cli_btn', 'preset_desktop_btn', 'reset_form_btn',
    'open_global_config_btn', 'language_select', 'encrypt_api_key', 'mask_api_key',
];

function normalizeLanguageCode(raw) {
    const v = (raw || '').trim();
    if (!v) return '';
    if (!/^[A-Za-z0-9_-]{1,32}$/.test(v)) return '';
    return v;
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

function maskApiKey(raw) {
    const v = (raw || '').trim();
    if (!v) return '';
    if (v.length <= 8) return '*'.repeat(v.length);
    return `${v.slice(0, 4)}${'*'.repeat(Math.max(4, v.length - 8))}${v.slice(-4)}`;
}

function getApiKeyInputValue() {
    const el = document.getElementById('api_key');
    if (!el) return '';
    if (el.dataset.masked === '1') return (sharedConfig.api_key || '').trim();
    return el.value.trim();
}

function isMaskedApiEchoState() {
    const el = document.getElementById('api_key');
    if (!el) return false;
    if (el.dataset.masked !== '1') return false;
    const shown = (el.value || '').trim();
    const stored = (sharedConfig.api_key || '').trim();
    // Strong signal of masked echo from cache: shown value equals stored value in masked view.
    return !!shown && shown === stored;
}

function isStrictGeneratedMaskedApiKey(raw) {
    const v = (raw || '').trim();
    if (!v) return false;
    // Exact generated patterns only:
    // 1) all stars (short keys)
    // 2) first4 + stars(>=4) + last4 (long keys)
    if (/^\*+$/.test(v)) return true;
    return /^.{4}\*{4,}.{4}$/.test(v);
}

function shouldResolveMaskedApiKey() {
    if (!sharedConfig.mask_api_key) return false;
    return sharedConfig.api_key_is_masked
        || isMaskedApiEchoState()
        || isStrictGeneratedMaskedApiKey(sharedConfig.api_key);
}

function renderApiKeyInput() {
    const el = document.getElementById('api_key');
    const editBtn = document.getElementById('api_key_edit_btn');
    if (!el) return;
    const current = (sharedConfig.api_key || '').trim();
    const maskedView = !!sharedConfig.mask_api_key && !!current && !apiKeyEditMode;
    if (maskedView) {
        const display = sharedConfig.api_key_is_masked ? current : maskApiKey(current);
        el.type = 'text';
        el.readOnly = true;
        el.value = display;
        el.dataset.masked = '1';
    } else {
        el.type = 'text';
        el.readOnly = false;
        el.value = current;
        el.dataset.masked = '0';
    }
    if (editBtn) {
        editBtn.style.display = (sharedConfig.mask_api_key && !!current && !apiKeyEditMode) ? '' : 'none';
        editBtn.disabled = !sharedConfig.mask_api_key || !current;
    }
}

async function fetchPlainApiKeyFromServer() {
    const resp = await fetch('/ui_cache/api_key_plain');
    if (!resp.ok) throw new Error(`api key fetch failed: ${resp.status}`);
    const data = await resp.json();
    return (data && typeof data.api_key === 'string') ? data.api_key.trim() : '';
}

async function beginEditApiKey() {
    if (!sharedConfig.mask_api_key) return;
    try {
        const mustFetchPlain = shouldResolveMaskedApiKey();
        if (mustFetchPlain) {
            const prevMaskedValue = (sharedConfig.api_key || '').trim();
            const plain = await fetchPlainApiKeyFromServer();
            if (!plain && prevMaskedValue) {
                throw new Error('empty plain api key from server while masked value exists');
            }
            sharedConfig.api_key = plain;
            sharedConfig.api_key_is_masked = false;
        }
        apiKeyEditMode = true;
        renderApiKeyInput();
        const editInput = document.getElementById('api_key');
        if (editInput) {
            editInput.focus();
            const len = editInput.value.length;
            editInput.setSelectionRange(len, len);
        }
    } catch (_e) {
        setStatus(txt('status_api_key_edit_failed', ''), 'status-danger');
    }
}

async function resolveApiKeyForRequest(forceResolve, silent) {
    const force = forceResolve === true;
    const quiet = silent === true;
    const shouldResolve = force || shouldResolveMaskedApiKey();
    if (!shouldResolve) return true;
    try {
        const prevMaskedValue = (sharedConfig.api_key || '').trim();
        const plain = await fetchPlainApiKeyFromServer();
        if (!plain && prevMaskedValue) {
            throw new Error('empty plain api key from server while masked value exists');
        }
        sharedConfig.api_key = plain;
        sharedConfig.api_key_is_masked = false;
        renderApiKeyInput();
        return true;
    } catch (_e) {
        if (!quiet) setStatus(txt('status_api_key_fetch_failed', ''), 'status-danger');
        return false;
    }
}

function reMaskApiKeyIfNeeded() {
    const el = document.getElementById('api_key');
    if (!el) return;
    if (sharedConfig.mask_api_key) {
        // Only persist when user was actually editing; never persist masked display text.
        if (el.dataset.masked !== '1') {
            sharedConfig.api_key = el.value.trim();
            sharedConfig.api_key_is_masked = false;
        }
        apiKeyEditMode = false;
    }
    updateSharedConfigFromInputs();
    scheduleUiCacheSave();
    renderApiKeyInput();
}

async function suggestProjectSlug(projectName, goal) {
    try {
        const apiReady = await resolveApiKeyForRequest(false, true);
        if (!apiReady) return `project-${Date.now()}`;
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
        history_max_messages: '',
        history_max_chars: '',
        release_gate_threshold: '',
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
    const apiKeyInput = document.getElementById('api_key');
    const isMaskedDisplay = apiKeyInput?.dataset.masked === '1';
    const apiKeyValue = isMaskedDisplay
        ? (sharedConfig.api_key || '').trim()
        : getApiKeyInputValue();
    const apiKeyIsMasked = isMaskedDisplay ? !!sharedConfig.api_key_is_masked : false;
    sharedConfig = {
        api_key: apiKeyValue,
        base_url: document.getElementById('base_url').value.trim(),
        model: document.getElementById('model').value.trim(),
        language: normalizeLanguageCode(document.getElementById('language_select')?.value || currentLanguage),
        encrypt_api_key: !!document.getElementById('encrypt_api_key')?.checked,
        mask_api_key: !!document.getElementById('mask_api_key')?.checked,
        api_key_is_masked: apiKeyIsMasked,
    };
}

function applySharedConfigToInputs() {
    document.getElementById('encrypt_api_key').checked = !!sharedConfig.encrypt_api_key;
    document.getElementById('mask_api_key').checked = !!sharedConfig.mask_api_key;
    apiKeyEditMode = false;
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
            hint.textContent = txt('readonly_backend_offline', 'Read-only mode: backend offline.');
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
            'Backend offline: communication actions are hidden and all input fields are read-only.'
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
        cacheProjectLogs = (data.project_logs && typeof data.project_logs === 'object') ? data.project_logs : {};
        cacheProjectUiState = (data.project_ui_state && typeof data.project_ui_state === 'object') ? data.project_ui_state : {};
        if (data.shared_config && typeof data.shared_config === 'object') {
            const maskedFlag = !!data.shared_config.api_key_is_masked;
            sharedConfig = {
                api_key: data.shared_config.api_key || '',
                base_url: data.shared_config.base_url || 'https://api.deepseek.com',
                model: data.shared_config.model || 'deepseek-reasoner',
                language: normalizeLanguageCode(data.shared_config.language || ''),
                encrypt_api_key: !!data.shared_config.encrypt_api_key,
                mask_api_key: maskedFlag ? true : !!data.shared_config.mask_api_key,
                api_key_is_masked: maskedFlag,
            };
            apiKeyEditMode = false;
        }
        return true;
    } catch (_e) {
        return false;
    }
}

function scheduleUiCacheSave() {
    if (uiCacheSaveTimer) clearTimeout(uiCacheSaveTimer);
    uiCacheSaveTimer = setTimeout(async () => {
        const payload = {
            shared_config: sharedConfig,
            project_logs: cacheProjectLogs,
            project_ui_state: cacheProjectUiState,
        };
        try {
            await fetch('/ui_cache', {
                method: 'PUT',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(payload),
            });
        } catch (_e) {}
    }, 250);
}

async function persistSharedConfigNow() {
    updateSharedConfigFromInputs();
    try {
        const resp = await fetch('/ui_cache', {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ shared_config: sharedConfig }),
        });
        return resp.ok;
    } catch (_e) {
        return false;
    }
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
    return { ...defaultBucketUiState(), ...raw };
}

function writeBucketUiState(bucket, patch) {
    const all = loadProjectUiStateMap();
    all[bucket] = { ...readBucketUiState(bucket), ...(patch || {}) };
    saveProjectUiStateMap(all);
}

function renderClarifyQuestions(questions) {
    const form = document.getElementById('clarify_form');
    form.innerHTML = '';
    (questions || []).forEach(q => {
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
            div.appendChild(input);
        }
        form.appendChild(div);
    });
}

function renderUiForViewBucket() {
    const bucket = viewLogBucket();
    const state = readBucketUiState(bucket);
    const status = document.getElementById('status');
    status.textContent = state.status_text || txt('status_waiting', '');
    status.className = 'status-box ' + (state.status_class || '');
    const bar = document.getElementById('runbar');
    bar.dataset.state = state.run_state || 'idle';
    document.getElementById('runbar_text').textContent = fmt('runbar_line', '', { state: state.run_text || txt('run_idle', '') });
    document.getElementById('diagnostics').textContent = state.diagnostics_text || txt('diagnostics_none', '');
    if (state.diff_text) {
        const diffElem = document.getElementById('diff');
        const lines = state.diff_text.split('\n');
        diffElem.innerHTML = lines.map(line => {
            if (line.startsWith('+') && !line.startsWith('+++')) {
                return `<div class="diff-line-add">${escapeHtml(line)}</div>`;
            }
            if (line.startsWith('-') && !line.startsWith('---')) {
                return `<div class="diff-line-del">${escapeHtml(line)}</div>`;
            }
            return `<div class="diff-line-other">${escapeHtml(line)}</div>`;
        }).join('');
        document.getElementById('diff_section').style.display = 'block';
    } else {
        document.getElementById('diff_section').style.display = 'none';
        document.getElementById('diff').innerHTML = '';
    }
    renderClarifyQuestions(state.clarify_questions || []);
}

function loadProjectLogs() {
    return cacheProjectLogs || {};
}

function saveProjectLogs(obj) {
    cacheProjectLogs = (obj && typeof obj === 'object') ? obj : {};
    scheduleUiCacheSave();
}

function readLogForBucket(bucket) {
    const all = loadProjectLogs();
    const raw = all[bucket];
    return typeof raw === 'string' ? raw : '';
}

function writeLogForBucket(bucket, content) {
    const all = loadProjectLogs();
    // Hard cap to avoid unbounded browser/server cache growth.
    all[bucket] = (content || '').slice(-300000);
    saveProjectLogs(all);
}

function renderCurrentLogView() {
    const log = document.getElementById('log');
    log.textContent = readLogForBucket(viewLogBucket());
    log.scrollTop = log.scrollHeight;
}

function normalizeLogLine(text) {
    const raw = String(text || '');
    if (!raw.startsWith('[Model-Stream]')) return raw;
    const body = raw.slice('[Model-Stream]'.length).trim();
    if (!body) return raw;
    const compact = body
        .replace(/\\n|\\r|\\t/g, ' ')
        .replace(/\s+/g, ' ')
        .trim();
    if (!compact) return '[Model-Stream]';
    return `[Model-Stream] ${compact.length > 220 ? `${compact.slice(0, 220)}...` : compact}`;
}

function appendLog(text) {
    const line = normalizeLogLine(text);
    const bucket = currentLogBucket();
    const existing = readLogForBucket(bucket);
    writeLogForBucket(bucket, existing + line + "\n");
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
    return readBucketUiState(currentLogBucket()).run_state || 'idle';
}

function hasPendingClarify(bucket) {
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
        setRunState('interrupted', txt('run_interrupted', ''));
        setStatus(txt('status_backend_disconnected', ''), 'status-danger');
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
    clearStopAckTimer();
    stopRequested = false;
    activeRunLogBucket = '';
    activeRunProjectLabel = '';
    runSessionActive = false;
    setRunningProjectIndicator(txt('running_project_none', ''));
    setProjectControlsDisabled(false);
    applyReadOnlyMode();
    setRunState('idle', txt('run_idle', ''));
    document.getElementById('start_btn').disabled = false;
    document.getElementById('stop_btn').disabled = true;
    if (!suppressStoppedAlert && !completionAlertShown) {
        completionAlertShown = true;
        alert(txt('alert_task_stopped', ''));
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
        history_max_messages: document.getElementById('history_max_messages').value.trim(),
        history_max_chars: document.getElementById('history_max_chars').value.trim(),
        release_gate_threshold: document.getElementById('release_gate_threshold').value.trim(),
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
    const fields = ['api_key', 'base_url', 'model', 'auto_revert_profile', 'precheck_cmd', 'history_max_messages', 'history_max_chars', 'release_gate_threshold', 'workspace', 'goal', 'eval_cmd', 'success_regex', 'remote', 'remote_url', 'branch'];
    fields.forEach(k => {
        if (typeof d[k] === 'string' && document.getElementById(k)) {
            document.getElementById(k).value = d[k];
        }
    });
    if (typeof d.language === 'string' && d.language) {
        const code = normalizeLanguageCode(d.language);
        if (code) sharedConfig.language = code;
    }
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
        ['section_clarify_title', 'section_clarify_title'],
        ['section_log_title', 'section_log_title'],
        ['hint_log_exports', 'hint_log_exports'],
        ['section_diff_title', 'section_diff_title'],
        ['section_global_title', 'section_global_title'],
        ['label_api_key', 'label_api_key'],
        ['label_model', 'label_model'],
        ['label_base_url', 'label_base_url'],
        ['label_language', 'label_language'],
        ['label_encrypt_api_key', 'label_encrypt_api_key'],
        ['label_mask_api_key', 'label_mask_api_key'],
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
        ['preset_webapp_btn', 'btn_preset_webapp'],
        ['preset_cli_btn', 'btn_preset_cli'],
        ['preset_desktop_btn', 'btn_preset_desktop'],
        ['reset_form_btn', 'btn_reset_form'],
        ['clarify_submit_btn', 'btn_clarify_submit'],
        ['clear_log_btn', 'btn_clear_log'],
        ['export_log_btn', 'btn_export_log'],
        ['export_snapshot_btn', 'btn_export_snapshot'],
        ['export_report_btn', 'btn_export_report'],
        ['api_key_edit_btn', 'btn_edit_api_key'],
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
        ['history_max_messages', 'ph_history_max_messages'],
        ['history_max_chars', 'ph_history_max_chars'],
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
}

function openGlobalConfigModal() {
    closeSidebarOnNarrow();
    document.getElementById('global_config_modal').style.display = 'flex';
}

function closeGlobalConfigModal() {
    document.getElementById('global_config_modal').style.display = 'none';
}

async function saveGlobalConfig() {
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
}

function setAutoSaveState(text) {
    document.getElementById('auto_save_state').textContent = text;
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
    workspace: ${escapeHtml(p.workspace || '-')}<br>
    ${escapeHtml(txt('project_meta_updated', ''))}: ${escapeHtml(dt)}<br>
    eval: ${escapeHtml(snap.eval_cmd || '-')}<br>
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

function selectProjectByCurrentForm() {
    const body = getFormData();
    const ws = (body.workspace || '').trim();
    const goal = (body.goal || '').trim();
    const items = loadProjects();
    const found = items.find(x =>
        (x.workspace || '').trim() === ws
        && (x.goal || '').trim() === goal
    );
    if (found) {
        renderProjectSelector(found.id);
        document.getElementById('project_name').value = found.name || '';
        projectNameManualOverride = true;
        lastAutoProjectName = '';
    }
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
    saveProjectLogs(logs);
    const uiMap = loadProjectUiStateMap();
    delete uiMap[`project:${found.id}`];
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
        if (cfg && typeof cfg.history_max_messages === 'string') {
            document.getElementById('history_max_messages').value = cfg.history_max_messages;
        }
        if (cfg && typeof cfg.history_max_chars === 'string') {
            document.getElementById('history_max_chars').value = cfg.history_max_chars;
        }
        if (cfg && typeof cfg.release_gate_threshold === 'string') {
            document.getElementById('release_gate_threshold').value = cfg.release_gate_threshold;
        }
    } catch (_e) {}
}

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

async function refreshUiState() {
    try {
        const resp = await fetch('/ui_state');
        if (!resp.ok) return;
        markBackendAlive();
        const data = await resp.json();
        renderResumeBanner(data);
        if (data.runtime && data.runtime.running) {
            const label = (data.runtime.last_workspace || '').trim() || txt('running_backend_task', '');
            if (!activeRunLogBucket) {
                activeRunLogBucket = `runtime:${label}`;
            }
            runSessionActive = true;
            setRunningProjectIndicator(label);
            setProjectControlsDisabled(true);
            applyReadOnlyMode();
            document.getElementById('start_btn').disabled = true;
            document.getElementById('stop_btn').disabled = false;
            document.getElementById('revert_btn').disabled = false;
            if (hasPendingClarify(currentLogBucket())) {
                setStatus(txt('status_need_clarify', ''), 'status-warn');
                setRunState('waiting_clarify', txt('run_waiting_clarify', ''));
            } else {
                setStatus(txt('status_running_detected', ''), 'status-warn');
                setRunState('running', txt('run_running', ''));
            }
        } else {
            syncStoppedStateIfNeeded(txt('sync_reason_backend_stopped', ''));
            setProjectControlsDisabled(false);
            setRunningProjectIndicator(txt('running_project_none', ''));
            runSessionActive = false;
            applyReadOnlyMode();
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

function handleEvent(evt) {
    switch (evt.type) {
        case 'log':
            if (currentRunState() === 'waiting_clarify' && !hasPendingClarify(currentLogBucket())) {
                setRunState('running', txt('run_running', ''));
            }
            appendLog(evt.line);
            updateDiagnosticsFromLog(evt.line);
            break;
        case 'diff':
            renderDiff(evt.diff);
            break;
        case 'need_clarify':
            showClarify(evt.questions);
            break;
        case 'done':
            clearStopAckTimer();
            activeRunLogBucket = '';
            activeRunProjectLabel = '';
            runSessionActive = false;
            setRunningProjectIndicator(txt('running_project_none', ''));
            setProjectControlsDisabled(false);
            applyReadOnlyMode();
            if (evt.success) {
                setRunState('success', txt('run_success', ''));
                setStatus(fmt('status_done_success', '', { message: evt.message }), 'status-ok');
                if (!completionAlertShown) {
                    completionAlertShown = true;
                    alert(fmt('alert_done_success', '', { message: evt.message }));
                }
            } else if (stopRequested || isInterruptedMessage(evt.message)) {
                setRunState('interrupted', txt('run_interrupted', ''));
                setStatus(fmt('status_done_interrupted', '', { message: evt.message }), 'status-warn');
                if (!completionAlertShown) {
                    completionAlertShown = true;
                    alert(fmt('alert_done_interrupted', '', { message: evt.message }));
                }
            } else {
                setRunState('failed', txt('run_failed', ''));
                setStatus(fmt('status_done_failed', '', { message: evt.message }), 'status-danger');
                if (!completionAlertShown) {
                    completionAlertShown = true;
                    alert(fmt('alert_done_failed', '', { message: evt.message }));
                }
            }
            stopRequested = false;
            document.getElementById('start_btn').disabled = false;
            document.getElementById('stop_btn').disabled = true;
            break;
    }
}

function showClarify(questions) {
    const bucket = currentLogBucket();
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
    const apiReady = await resolveApiKeyForRequest();
    if (!apiReady) return;
    const synced = await persistSharedConfigNow();
    if (!synced) {
        setStatus(txt('status_start_shared_config_sync_failed', ''), 'status-danger');
        return;
    }
    let body = getFormData();
    if (!body.workspace) {
        const ok = await saveCurrentProject();
        ensureWorkspaceForCurrentProject();
        body = getFormData();
        if (!ok || !body.workspace) {
            setStatus(txt('status_start_alloc_workspace_failed', ''), 'status-danger');
            return;
        }
    }
    if (!body.api_key) {
        setStatus(txt('status_start_api_key_empty', ''), 'status-danger');
        return;
    }
    try {
        activeRunLogBucket = currentLogBucket();
        activeRunProjectLabel = currentProjectLabel();
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
        setRunningProjectIndicator(activeRunProjectLabel);
        setProjectControlsDisabled(true);
        runSessionActive = true;
        applyReadOnlyMode();
        setRunState('running', txt('run_running', ''));
        stopRequested = false;
        completionAlertShown = false;
        document.getElementById('start_btn').disabled = true;
        document.getElementById('stop_btn').disabled = false;
        document.getElementById('revert_btn').disabled = false;
    } catch (e) {
        activeRunLogBucket = '';
        activeRunProjectLabel = '';
        runSessionActive = false;
        setRunningProjectIndicator(txt('running_project_none', ''));
        setProjectControlsDisabled(false);
        applyReadOnlyMode();
        setStatus(`${txt('status_start_failed_prefix', '')}${e}`, 'status-danger');
    }
}

async function resumeSession() {
    try {
        ensureWorkspaceForCurrentProject();
        if (isReadOnlyView()) {
            setStatus(txt('status_readonly_view', ''), 'status-danger');
            return;
        }
        const apiReady = await resolveApiKeyForRequest();
        if (!apiReady) return;
        const selectedProjectId = document.getElementById('project_selector')?.value || '';
        if (!selectedProjectId) {
            setStatus(txt('status_need_select_project', ''), 'status-danger');
            return;
        }
        const synced = await persistSharedConfigNow();
        if (!synced) {
            setStatus(txt('status_shared_config_sync_failed', ''), 'status-danger');
            return;
        }
        let body = getFormData();
        if (!body.workspace) {
            const ok = await saveCurrentProject();
            ensureWorkspaceForCurrentProject();
            body = getFormData();
            if (!ok || !body.workspace) {
                setStatus(txt('status_resume_alloc_workspace_failed', ''), 'status-danger');
                return;
            }
        }
        activeRunLogBucket = currentLogBucket();
        activeRunProjectLabel = currentProjectLabel();
        const resp = await fetch('/resume', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                workspace: body.workspace,
                project_id: selectedProjectId,
            }),
        });
        if (!resp.ok) {
            const msg = await resp.text();
            throw new Error(`${resp.status} ${msg}`);
        }
        setStatus(txt('status_resumed', ''), 'status-warn');
        setRunningProjectIndicator(activeRunProjectLabel);
        setProjectControlsDisabled(true);
        runSessionActive = true;
        applyReadOnlyMode();
        setRunState('running', txt('run_running', ''));
        stopRequested = false;
        completionAlertShown = false;
        document.getElementById('start_btn').disabled = true;
        document.getElementById('stop_btn').disabled = false;
    } catch (e) {
        activeRunLogBucket = '';
        activeRunProjectLabel = '';
        runSessionActive = false;
        setRunningProjectIndicator(txt('running_project_none', ''));
        setProjectControlsDisabled(false);
        applyReadOnlyMode();
        setStatus(`${txt('status_resume_failed_prefix', '')}${e}`, 'status-danger');
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
            setRunState('interrupted', txt('run_interrupted_timeout', ''));
            setStatus(txt('status_stop_ack_timeout', ''), 'status-warn');
        }, 6000);
        setRunState('stopping', txt('run_stopping', ''));
        if (ack.includes('channel unavailable')) {
            setStatus(txt('status_stop_backend_done', ''), 'status-warn');
        } else {
            setStatus(txt('status_stop_waiting_ack', ''), 'status-warn');
        }
        document.getElementById('start_btn').disabled = false;
        document.getElementById('stop_btn').disabled = true;
    } catch (e) {
        setRunState('interrupted', txt('run_interrupted', ''));
        setStatus(`${txt('status_stop_failed_prefix', '')}${e}`, 'status-danger');
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
    const formData = new FormData(document.getElementById('clarify_form'));
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
        writeBucketUiState(currentLogBucket(), { clarify_questions: [] });
        setRunState('running', txt('run_running', ''));
        setStatus(txt('status_clarify_submitted_waiting', ''), 'status-warn');
        appendLog(txt('log_submit_clarify', ''));
        if (viewLogBucket() === currentLogBucket()) renderUiForViewBucket();
    } catch (err) {
        setStatus(`${txt('status_clarify_failed_prefix', '')}${err}`, 'status-danger');
    }
}

function renderDiff(diffText) {
    const bucket = currentLogBucket();
    writeBucketUiState(bucket, { diff_text: diffText || '' });
    if (viewLogBucket() !== bucket) return;
    const diffElem = document.getElementById('diff');
    const lines = diffText.split('\n');
    diffElem.innerHTML = lines.map(line => {
        if (line.startsWith('+') && !line.startsWith('+++')) {
            return `<div class="diff-line-add">${escapeHtml(line)}</div>`;
        }
        if (line.startsWith('-') && !line.startsWith('---')) {
            return `<div class="diff-line-del">${escapeHtml(line)}</div>`;
        }
        return `<div class="diff-line-other">${escapeHtml(line)}</div>`;
    }).join('');
    document.getElementById('diff_section').style.display = 'block';
}

function escapeHtml(text) {
    return text
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;');
}

function applyPreset(kind) {
    const goal = document.getElementById('goal');
    const evalCmd = document.getElementById('eval_cmd');

    if (kind === 'webapp') {
        goal.value = txt('preset_goal_webapp', '');
        evalCmd.value = 'bash scripts/run_tests.sh';
    } else if (kind === 'cli') {
        goal.value = txt('preset_goal_cli', '');
        evalCmd.value = 'bash scripts/run_tests.sh';
    } else if (kind === 'desktop') {
        goal.value = txt('preset_goal_desktop', '');
        evalCmd.value = 'bash scripts/run_tests.sh';
    }
    autoFillProjectNameFromGoal(false);
    markProjectDirty();
    scheduleDraftSave();
}

function resetForm() {
    applyFormData(defaultFormData());
    projectNameManualOverride = false;
    lastAutoProjectName = '';
    autoFillProjectNameFromGoal(true);
    markProjectDirty();
    scheduleDraftSave();
}

function bindAutoSave() {
    const ids = ['auto_revert_profile', 'precheck_cmd', 'history_max_messages', 'history_max_chars', 'release_gate_threshold', 'goal', 'eval_cmd', 'success_regex', 'remote', 'remote_url', 'branch'];
    ids.forEach(id => {
        const el = document.getElementById(id);
        if (!el) return;
        el.addEventListener('input', () => { markProjectDirty(); scheduleDraftSave(); });
        el.addEventListener('change', () => { markProjectDirty(); scheduleDraftSave(); });
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
    document.getElementById('preset_webapp_btn').addEventListener('click', () => applyPreset('webapp'));
    document.getElementById('preset_cli_btn').addEventListener('click', () => applyPreset('cli'));
    document.getElementById('preset_desktop_btn').addEventListener('click', () => applyPreset('desktop'));
    document.getElementById('reset_form_btn').addEventListener('click', resetForm);
    document.getElementById('project_new_btn').addEventListener('click', createNewProject);
    document.getElementById('project_save_btn').addEventListener('click', () => { saveCurrentProject(); });
    document.getElementById('project_load_btn').addEventListener('click', loadSelectedProject);
    document.getElementById('project_delete_btn').addEventListener('click', () => { deleteSelectedProject(); });
    document.getElementById('project_selector').addEventListener('change', syncProjectNameFromSelection);
    document.getElementById('project_search').addEventListener('input', () => {
        projectSearchKeyword = document.getElementById('project_search').value || '';
        renderProjectAccordion();
    });
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
    document.getElementById('api_key_edit_btn').addEventListener('click', beginEditApiKey);
    document.getElementById('api_key').addEventListener('input', () => {
        if (document.getElementById('api_key').dataset.masked === '1') return;
        updateSharedConfigFromInputs();
        scheduleUiCacheSave();
    });
    document.getElementById('api_key').addEventListener('blur', reMaskApiKeyIfNeeded);
    document.getElementById('mask_api_key').addEventListener('change', async () => {
        updateSharedConfigFromInputs();
        const needResolveOnUnmask = sharedConfig.api_key_is_masked
            || isMaskedApiEchoState()
            || isStrictGeneratedMaskedApiKey(sharedConfig.api_key);
        if (!sharedConfig.mask_api_key && needResolveOnUnmask) {
            const apiReady = await resolveApiKeyForRequest(true);
            if (!apiReady) {
                sharedConfig.mask_api_key = true;
                document.getElementById('mask_api_key').checked = true;
            }
            updateSharedConfigFromInputs();
        }
        if (sharedConfig.mask_api_key) apiKeyEditMode = false;
        renderApiKeyInput();
        scheduleUiCacheSave();
    });
    document.getElementById('encrypt_api_key').addEventListener('change', () => {
        updateSharedConfigFromInputs();
        scheduleUiCacheSave();
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
    await refreshProjectsFromServer();
    renderProjectSelector('');
    bindEvents();
    bindAutoSave();
    setProjectDraftState();
    setRunningProjectIndicator(txt('running_project_none', ''));
    setProjectControlsDisabled(false);
    applyReadOnlyMode();
    updateGoRunningProjectButton();
    document.getElementById('stop_btn').disabled = true;
    document.getElementById('revert_btn').disabled = true;
    setRunState('idle', txt('run_idle', ''));
    setAutoSaveState(fmt('autosave_project_idle', '', {}));
    autoFillProjectNameFromGoal(false);
    markProjectClean();
    applySharedConfigToInputs();
    autoOpenLatestProject();
    renderCurrentLogView();
    renderUiForViewBucket();
    await loadProjectConfigForWorkspace();
    await refreshUiState();
    await refreshMetrics();
    setInterval(refreshUiState, UI_STATE_SYNC_MS);
    setInterval(refreshMetrics, 5000);
    setInterval(monitorRealtimeChannel, 2000);
    // Prefer SSE for low-latency updates, fallback to long-polling only on failure.
    startEventStream();
    window.addEventListener('beforeunload', closeEventStream);
}

init();
