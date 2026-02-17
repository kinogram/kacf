let lastEventId = 0;
let polling = false;
let saveTimer = null;
let projectAutoSaveTimer = null;
let projectAutoSaveInFlight = false;
let stopRequested = false;
let stopDisplayedAsStopped = false; // UI already finalized stop; ignore subsequent /done state flip.
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
let guestMode = false;
let backendReconnectReloading = false;
const POLL_MS_ACTIVE = 1000;
const POLL_MS_IDLE = 3000;
const UI_STATE_SYNC_MS = 3000;
const SSE_STALE_MS = 8000;
const FALLBACK_LOG_DEDUP_MS = 30000;
const BACKEND_FAILURE_THRESHOLD = 3;
const SSE_ERROR_LIMIT = 6;
const SSE_CONNECT_GRACE_MS = 10000;
const DEFAULT_LOG_MAX_CHARS = 20000;
const LARGE_CHAR_LIMIT_WARNING_THRESHOLD = 30000;
const GLOBAL_OPTION_DEFAULTS = {
    auto_resume_attempts: '',
    stop_after_minutes: '',
    history_max_messages: '40',
    history_max_chars: '70000',
    log_max_chars: String(DEFAULT_LOG_MAX_CHARS),
};
const GLOBAL_OPTION_INPUT_ID_BY_KEY = {
    auto_resume_attempts: 'global_auto_resume_attempts',
    stop_after_minutes: 'global_stop_after_minutes',
    history_max_messages: 'global_history_max_messages',
    history_max_chars: 'global_history_max_chars',
    log_max_chars: 'global_log_max_chars',
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
let WORKSPACE_ROOT = './autocoding_data/workspaces';
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
window.KACF = window.KACF || {};
const BACKEND_COMM_BUTTON_IDS = [
    'start_btn',
    'resume_btn',
    'stop_btn',
    'revert_btn',
    'push_btn',
    'clarify_submit_btn',
    'view_diff_btn',
    'project_new_btn',
    'project_save_btn',
    'project_load_btn',
    'project_delete_btn',
    'open_global_config_btn',
    'save_global_config_btn',
    'vm_provision_btn',
    'vm_refresh_btn',
    'vm_ready_btn',
    'vm_bootstrap_btn',
    'vm_start_btn',
    'vm_stop_btn',
    'vm_delete_btn',
    'vm_refresh_logs_btn',
    'vm_snapshot_refresh_btn',
    'vm_snapshot_create_btn',
    'vm_snapshot_apply_btn',
    'vm_snapshot_delete_btn',
    'vm_clone_btn',
    'vm_exec_btn',
    'vm_exec_cancel_btn',
    'vm_exec_enqueue_btn',
    'vm_exec_batch_enqueue_btn',
    'vm_exec_profile_enqueue_btn',
    'vm_exec_profile_preview_btn',
    'vm_exec_custom_profile_load_btn',
    'vm_exec_custom_profile_save_btn',
    'vm_exec_custom_profile_delete_btn',
    'vm_self_debug_preview_btn',
    'vm_self_debug_start_btn',
    'vm_self_debug_runs_refresh_btn',
    'vm_self_debug_stats_btn',
    'vm_self_debug_detail_btn',
    'vm_self_debug_pause_btn',
    'vm_self_debug_resume_btn',
    'vm_self_debug_stop_btn',
    'vm_self_debug_rules_load_btn',
    'vm_self_debug_rules_save_btn',
    'vm_exec_run_next_btn',
    'vm_exec_queue_refresh_btn',
    'vm_exec_queue_cancel_btn',
];
const PROJECT_CONTROL_IDS = [
    'project_name',
    'project_new_btn',
    'project_save_btn',
    'project_load_btn',
    'project_delete_btn',
];
const CONFIG_EDIT_IDS = [
    'api_key', 'model', 'base_url', 'workspace',
    'unattended_mode',
    'goal', 'remote', 'remote_url', 'branch', 'git_user_name', 'git_user_email',
    'open_global_config_btn', 'language_select',
];

// Debug log reporting: lets us diagnose Android/tablet issues without remote DevTools.
// Best-effort only; failures are ignored.
let debugLogLastSentAt = 0;
let debugLogBurst = 0;
function postDebugLog(payload) {
    const now = Date.now();
    if (now - debugLogLastSentAt > 5000) {
        debugLogBurst = 0;
        debugLogLastSentAt = now;
    }
    debugLogBurst++;
    if (debugLogBurst > 6) return; // avoid flooding
    try {
        fetch('/debug/client_logs', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload || {}),
        }).catch(() => {});
    } catch (_e) {}
}
window.addEventListener('error', (ev) => {
    const err = ev && ev.error;
    postDebugLog({
        level: 'error',
        message: String(ev && ev.message ? ev.message : 'window.error'),
        href: String(location && location.href ? location.href : ''),
        user_agent: String(navigator && navigator.userAgent ? navigator.userAgent : ''),
        stack: String(err && err.stack ? err.stack : ''),
    });
});
window.addEventListener('unhandledrejection', (ev) => {
    const r = ev && ev.reason;
    postDebugLog({
        level: 'error',
        message: String(r && r.message ? r.message : r ? r : 'unhandledrejection'),
        href: String(location && location.href ? location.href : ''),
        user_agent: String(navigator && navigator.userAgent ? navigator.userAgent : ''),
        stack: String(r && r.stack ? r.stack : ''),
    });
});

// Startup ping: confirms client -> server debug log path is working on the device.
postDebugLog({
    level: 'info',
    message: 'debug_log_online',
    href: String(location && location.href ? location.href : ''),
    user_agent: String(navigator && navigator.userAgent ? navigator.userAgent : ''),
    stack: '',
});

let sidebarLayoutDebugSent = false;
function debugSidebarLayoutOnce(narrow, t, m) {
    if (sidebarLayoutDebugSent) return;
    sidebarLayoutDebugSent = true;
    postDebugLog({
        level: 'info',
        message: 'applySidebarLayout_called',
        href: String(location && location.href ? location.href : ''),
        user_agent: String(navigator && navigator.userAgent ? navigator.userAgent : ''),
        stack: JSON.stringify({
            narrow: !!narrow,
            vw: window.innerWidth,
            vh: window.innerHeight,
            sidebarCollapsed: !!sidebarCollapsed,
            sidebarMobileOpen: !!sidebarMobileOpen,
            hasToggleBtn: !!t,
            hasMobileBtn: !!m,
        }),
    });
}

let sidebarIconDebugReported = false;
function maybeReportIconVisibility(btn, id, narrow) {
    if (!btn || sidebarIconDebugReported) return;
    // Defer until after layout/paint.
    requestAnimationFrame(() => {
        if (sidebarIconDebugReported) return;
        const cs = window.getComputedStyle(btn);
        const r = btn.getBoundingClientRect();
        const txt = (btn.textContent || '').trim();
        const invisible =
            !txt ||
            cs.display === 'none' ||
            cs.visibility === 'hidden' ||
            Number(cs.opacity || '1') === 0 ||
            r.width < 8 ||
            r.height < 8 ||
            cs.color === 'transparent' ||
            cs.color === 'rgba(0, 0, 0, 0)' ||
            cs.color === 'rgba(0,0,0,0)';
        if (!invisible) return;
        sidebarIconDebugReported = true;
        postDebugLog({
            level: 'warn',
            message: `sidebar icon not visible: ${id}`,
            href: String(location && location.href ? location.href : ''),
            user_agent: String(navigator && navigator.userAgent ? navigator.userAgent : ''),
            stack: JSON.stringify({
                narrow: !!narrow,
                sidebarCollapsed: !!sidebarCollapsed,
                sidebarMobileOpen: !!sidebarMobileOpen,
                text: txt,
                className: btn.className || '',
                display: cs.display,
                visibility: cs.visibility,
                opacity: cs.opacity,
                color: cs.color,
                rect: { x: r.x, y: r.y, w: r.width, h: r.height },
            }),
        });
    });
}

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

function maybeWarnLargeCharLimit(source) {
    const logLimit = logMaxCharsSetting();
    if (logLimit <= LARGE_CHAR_LIMIT_WARNING_THRESHOLD) {
        return;
    }
    setStatus(fmt('status_large_char_limit_warning', '', {
        source: source || txt('label_global_config', ''),
        threshold: LARGE_CHAR_LIMIT_WARNING_THRESHOLD,
        log_limit: logLimit,
    }), 'status-warn');
}

function truncateTailChars(raw, maxChars) {
    const text = String(raw || '');
    if (maxChars <= 0) return '';
    if (text.length <= maxChars) return text;
    return text.slice(text.length - maxChars);
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
        out[String(k)] = { ...entry };
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

function buildIconSvg(name) {
    // Build SVG elements via DOM APIs (more reliable than innerHTML parsing on Safari).
    const NS = 'http://www.w3.org/2000/svg';
    const svg = document.createElementNS(NS, 'svg');
    svg.setAttribute('viewBox', '0 0 24 24');
    // Set explicit dimensions + paint attributes to avoid CSS inheritance quirks on mobile browsers.
    svg.setAttribute('width', '18');
    svg.setAttribute('height', '18');
    svg.setAttribute('fill', 'none');
    svg.setAttribute('stroke', 'currentColor');
    svg.setAttribute('stroke-width', '2');
    svg.setAttribute('stroke-linecap', 'round');
    svg.setAttribute('stroke-linejoin', 'round');
    svg.setAttribute('aria-hidden', 'true');

    if (name === 'gear') {
        // Lucide "settings" style cog: path + center circle.
        const path = document.createElementNS(NS, 'path');
        path.setAttribute('d', 'M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.38a2 2 0 0 0-.73-2.73l-.15-.09a2 2 0 0 1-1-1.74v-.51a2 2 0 0 1 1-1.72l.15-.1a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z');
        path.setAttribute('fill', 'none');
        path.setAttribute('stroke', 'currentColor');
        path.setAttribute('stroke-width', '2');
        path.setAttribute('stroke-linecap', 'round');
        path.setAttribute('stroke-linejoin', 'round');
        svg.appendChild(path);
        const circle = document.createElementNS(NS, 'circle');
        circle.setAttribute('cx', '12');
        circle.setAttribute('cy', '12');
        circle.setAttribute('r', '3');
        circle.setAttribute('fill', 'none');
        circle.setAttribute('stroke', 'currentColor');
        circle.setAttribute('stroke-width', '2');
        svg.appendChild(circle);
        return svg;
    }

    if (name === 'user') {
        const NS = 'http://www.w3.org/2000/svg';
        const circle = document.createElementNS(NS, 'circle');
        circle.setAttribute('cx', '12');
        circle.setAttribute('cy', '8');
        circle.setAttribute('r', '4');
        circle.setAttribute('fill', 'none');
        circle.setAttribute('stroke', 'currentColor');
        circle.setAttribute('stroke-width', '2');
        svg.appendChild(circle);
        const path = document.createElementNS(NS, 'path');
        path.setAttribute('d', 'M4 21v-1a7 7 0 0 1 7-7h2a7 7 0 0 1 7 7v1');
        path.setAttribute('fill', 'none');
        path.setAttribute('stroke', 'currentColor');
        path.setAttribute('stroke-width', '2');
        path.setAttribute('stroke-linecap', 'round');
        path.setAttribute('stroke-linejoin', 'round');
        svg.appendChild(path);
        return svg;
    }

    if (name === 'link') {
        const first = document.createElementNS(NS, 'path');
        first.setAttribute('d', 'M10 13a5 5 0 0 0 7.07 0l1.41-1.41a5 5 0 0 0-7.07-7.07L10 5');
        first.setAttribute('fill', 'none');
        first.setAttribute('stroke', 'currentColor');
        first.setAttribute('stroke-width', '2');
        first.setAttribute('stroke-linecap', 'round');
        first.setAttribute('stroke-linejoin', 'round');
        svg.appendChild(first);
        const second = document.createElementNS(NS, 'path');
        second.setAttribute('d', 'M14 11a5 5 0 0 0-7.07 0l-1.41 1.41a5 5 0 1 0 7.07 7.07L14 19');
        second.setAttribute('fill', 'none');
        second.setAttribute('stroke', 'currentColor');
        second.setAttribute('stroke-width', '2');
        second.setAttribute('stroke-linecap', 'round');
        second.setAttribute('stroke-linejoin', 'round');
        svg.appendChild(second);
        return svg;
    }

    const path = document.createElementNS(NS, 'path');
    const dMap = {
        'chevron-left': 'M15 6l-6 6 6 6',
        'chevron-right': 'M9 6l6 6-6 6',
        'x': 'M6 6l12 12M18 6l-12 12',
        'menu': 'M4 7h16M4 12h16M4 17h16',
        'shield': 'M12 2l7 4v6c0 5-3 9-7 10C8 21 5 17 5 12V6l7-4',
    };
    const d = dMap[name] || '';
    if (!d) return null;
    path.setAttribute('d', d);
    // Duplicate paint attributes on the path too (Safari can ignore svg-level stroke in some cases).
    path.setAttribute('fill', 'none');
    path.setAttribute('stroke', 'currentColor');
    path.setAttribute('stroke-width', '2');
    path.setAttribute('stroke-linecap', 'round');
    path.setAttribute('stroke-linejoin', 'round');
    svg.appendChild(path);
    return svg;
}

function setIconButton(btn, iconName, label) {
    if (!btn) return;
    btn.classList.add('btn-icon-only');
    // Clear previous content reliably.
    while (btn.firstChild) btn.removeChild(btn.firstChild);

    // Prefer SVG icons (DeepSeek-like) but keep ASCII fallback for maximum compatibility.
    const svg = buildIconSvg(iconName);
    if (svg) {
        const span = document.createElement('span');
        span.className = 'btn-icon';
        span.setAttribute('aria-hidden', 'true');
        span.appendChild(svg);
        btn.appendChild(span);
    } else {
        const fallbackMap = { menu: '|||', x: 'X', 'chevron-left': '<', 'chevron-right': '>' };
        btn.textContent = fallbackMap[iconName] || '·';
        btn.style.fontSize = '20px';
        btn.style.lineHeight = '1';
    }

    btn.title = label || '';
    if (label) btn.setAttribute('aria-label', label);
}

function setButtonWithIcon(btn, iconName, label) {
    if (!btn) return;
    // Clear previous content reliably.
    while (btn.firstChild) btn.removeChild(btn.firstChild);
    btn.classList.remove('btn-icon-only');

    const svg = buildIconSvg(iconName);
    if (svg) {
        const span = document.createElement('span');
        span.className = 'btn-icon';
        span.setAttribute('aria-hidden', 'true');
        span.appendChild(svg);
        btn.appendChild(span);
    }
    const text = document.createElement('span');
    text.className = 'btn-label';
    text.textContent = label || '';
    btn.appendChild(text);

    btn.title = '';
    if (label) btn.setAttribute('aria-label', label);
}

function syncTopbarHeightVar() {
    const topbar = document.getElementById('topbar');
    if (!topbar) return;
    const h = Math.max(0, Math.round(topbar.getBoundingClientRect().height));
    if (!h) return;
    document.documentElement.style.setProperty('--topbar-h', `${h}px`);
}

function applySidebarLayout() {
    syncTopbarHeightVar();
    const root = document.body;
    const narrow = isNarrowViewport();
    root.classList.toggle('sidebar-collapsed', !narrow && sidebarCollapsed);
    root.classList.toggle('sidebar-open', narrow && sidebarMobileOpen);
    const b = document.getElementById('sidebar_toggle_btn');
    debugSidebarLayoutOnce(narrow, b, null);
    if (!b) return;
    b.style.display = 'inline-flex';
    const label = narrow
        ? (sidebarMobileOpen ? txt('btn_sidebar_close', '') : txt('btn_sidebar_open', ''))
        : (sidebarCollapsed ? txt('btn_sidebar_expand', '') : txt('btn_sidebar_collapse', ''));
    const iconName = narrow
        ? (sidebarMobileOpen ? 'x' : 'menu')
        : (sidebarCollapsed ? 'chevron-right' : 'chevron-left');
    setIconButton(b, iconName, label);
    maybeReportIconVisibility(b, 'sidebar_toggle_btn', narrow);
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

function escapeHtml(text) {
    // Used by project list rendering; must be available before app_projects.js runs.
    return String(text || '')
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;');
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
        unattended_mode: false,
        workspace: '',
        goal: '',
        remote: 'origin',
        remote_url: '',
        branch: 'main',
        git_user_name: '',
        git_user_email: '',
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

function setTopbarHintText(el, text, opts) {
    if (!el) return;
    opts = opts || {};

    const innerClass = 'marquee-inner';
    let inner = el.querySelector(`:scope > span.${innerClass}`);
    if (!inner) {
        el.textContent = '';
        inner = document.createElement('span');
        inner.className = innerClass;
        el.appendChild(inner);
    }

    const next = String(text || '');
    const seg1Class = 'marquee-seg';
    const gapPx = 24;

    // If the rendered text is identical, avoid restarting the animation.
    const seg1Existing = inner.querySelector(`:scope > span.${seg1Class}`);
    const sameText = seg1Existing ? (seg1Existing.textContent === next) : (inner.textContent === next);
    if (!opts.force && sameText && !opts.allowResetWhenSameText) return;

    // Ensure we have exactly one segment for measurement and non-overflow rendering.
    inner.replaceChildren();
    const seg1 = document.createElement('span');
    seg1.className = seg1Class;
    seg1.textContent = next;
    inner.appendChild(seg1);

    el.classList.remove('marquee');
    el.style.removeProperty('--marquee-step');
    el.style.removeProperty('--marquee-duration');
    el.style.setProperty('--marquee-gap', `${gapPx}px`);

    // Defer measurement until layout is stable.
    requestAnimationFrame(() => {
        const maxW = Math.max(0, el.clientWidth);
        const w = Math.max(0, seg1.scrollWidth);
        const overflow = w - maxW;
        if (overflow <= 8) return;

        // Seamless marquee: duplicate the segment so the loop boundary has identical content.
        const seg2 = document.createElement('span');
        seg2.className = seg1Class;
        seg2.textContent = next;
        inner.appendChild(seg2);

        const step = w + gapPx;
        const duration = Math.max(6, Math.min(22, step / 35));
        el.classList.add('marquee');
        el.style.setProperty('--marquee-step', `${step}px`);
        el.style.setProperty('--marquee-duration', `${duration}s`);
    });
}

let topbarParts = { run: '', project: '', unattended: '' };

function renderTopbarCombined(opts) {
    const el = document.getElementById('topbar_line_text');
    if (!el) return;
    const line = fmt('topbar_combined_line', '', {
        run: String(topbarParts.run || ''),
        project: String(topbarParts.project || ''),
        unattended: String(topbarParts.unattended || ''),
    });
    setTopbarHintText(el, line, opts || {});
}

function setTopbarPart(kind, text, opts) {
    const k = String(kind || '');
    if (k === 'run') topbarParts.run = String(text || '');
    else if (k === 'project') topbarParts.project = String(text || '');
    else if (k === 'unattended') topbarParts.unattended = String(text || '');
    renderTopbarCombined(opts || {});
}

function refreshTopbarCombined() {
    renderTopbarCombined({ force: true, allowResetWhenSameText: true });
}

function setRunningProjectIndicator(label) {
    setTopbarPart(
        'project',
        fmt('running_project_line', '', { name: label || txt('running_project_none', '') }),
    );
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
        if (backendOffline || guestMode) {
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
    const locked = backendOffline || guestMode;
    const readOnly = locked || viewReadOnly;
    const lockConfigInputs = locked || runSessionActive || viewReadOnly;
    const lockProjectControls = locked || runSessionActive;
    const hint = document.getElementById('readonly_mode_text');
    if (hint) {
        if (guestMode) {
            hint.textContent = txt('readonly_guest_mode', '');
        } else if (backendOffline) {
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
    setOfflineInputLock(locked);
    const offlineBanner = document.getElementById('backend_offline_banner');
    if (offlineBanner) {
        if (guestMode) {
            offlineBanner.textContent = txt('banner_guest_mode', '');
            offlineBanner.style.display = 'block';
            offlineBanner.style.background = '#eef6ff';
            offlineBanner.style.borderColor = '#a7d3ff';
            offlineBanner.style.color = '#0b3a6b';
        } else {
            offlineBanner.textContent = txt('banner_backend_offline', '');
            offlineBanner.style.display = backendOffline ? 'block' : 'none';
            offlineBanner.style.background = '#ffe9e7';
            offlineBanner.style.borderColor = '#ffb8b2';
            offlineBanner.style.color = '#8a1f17';
        }
    }
    updateGoRunningProjectButton();
}

function setGuestModeLocked(on) {
    guestMode = !!on;
    applyReadOnlyMode();
}

window.KACF = window.KACF || {};
window.KACF.state = window.KACF.state || {};
window.KACF.state.setGuestModeLocked = setGuestModeLocked;

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

window.KACF = window.KACF || {};
window.KACF.state = {
    normalizeLanguageCode,
    parseBoundedInt,
    normalizeGlobalOptions,
    logMaxCharsSetting,
    maybeWarnLargeCharLimit,
    truncateTailChars,
    sanitizeLogContent,
    sanitizeProjectLogsMap,
    sanitizeProjectUiStateMap,
    touchBucket,
    pruneLogBuckets,
    pruneUiStateBuckets,
    languageLabel,
    isNarrowViewport,
    applySidebarLayout,
    setSidebarCollapsed,
    setSidebarMobileOpen,
    closeSidebarOnNarrow,
    choosePreferredLanguage,
    fetchLanguagesFromServer,
    loadCopyForLanguage,
    renderLanguageOptions,
    switchLanguage,
    initLanguagePack,
    txt,
    fmt,
    inferProjectNameFromGoal,
    autoFillProjectNameFromGoal,
    getApiKeyInputValue,
    renderApiKeyInput,
    suggestProjectSlug,
    workspaceLeaf,
    ensureUniqueWorkspace,
    defaultFormData,
    updateSharedConfigFromInputs,
    applySharedConfigToInputs,
    nowText,
    setRunningProjectIndicator,
    setRunActionButtons,
    setElementsDisabled,
    setProjectControlsDisabled,
    setConfigInputsDisabled,
    applyBackendOfflineMode,
    setOfflineInputLock,
    isReadOnlyView,
    applyReadOnlyMode,
    setTopbarPart,
    refreshTopbarCombined,
    setGuestModeLocked,
    updateGoRunningProjectButton,
    goToRunningProjectView,
    currentProjectLabel,
    adHocLogBucketFromForm,
    viewLogBucket,
    currentLogBucket,
};
