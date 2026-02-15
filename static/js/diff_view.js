(() => {
    // Minimal, standalone diff viewer: no app.js dependency.

    let COPY = {};
    let sharedConfig = { language: '' };
    let globalOptions = { stop_after_minutes: '', auto_resume_attempts: '' };
    let currentLanguage = '';

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

    function parseBoundedInt(raw, min, max, fallback) {
        const n = Number.parseInt(String(raw || '').trim(), 10);
        if (!Number.isFinite(n)) return fallback;
        if (n < min) return min;
        if (n > max) return max;
        return n;
    }

    function normalizeLanguageCode(raw) {
        const v = (raw || '').trim();
        if (!v) return '';
        if (!/^[A-Za-z0-9_-]{1,32}$/.test(v)) return '';
        return v;
    }

    function choosePreferredLanguage(available, preferred) {
        const list = Array.isArray(available) ? available : [];
        if (!list.length) return 'en';
        const want = normalizeLanguageCode(preferred);
        if (want && list.includes(want)) return want;
        const nav = (navigator.language || '').toLowerCase();
        const zh = list.find(x => String(x).toLowerCase().startsWith('zh'));
        const en = list.find(x => String(x).toLowerCase().startsWith('en'));
        if (nav.startsWith('zh') && zh) return zh;
        return en || list[0];
    }

    function qs(name) {
        try {
            const u = new URL(window.location.href);
            return u.searchParams.get(name) || '';
        } catch (_e) {
            return '';
        }
    }

    async function fetchUiCache() {
        try {
            const resp = await fetch('/ui_cache', { cache: 'no-store' });
            if (!resp.ok) return null;
            return await resp.json();
        } catch (_e) {
            return null;
        }
    }

    async function fetchLanguages() {
        try {
            const resp = await fetch('/assets/languages/list', { cache: 'no-store' });
            if (!resp.ok) return [];
            const data = await resp.json();
            return Array.isArray(data.languages) ? data.languages : [];
        } catch (_e) {
            return [];
        }
    }

    async function loadCopyForLanguage(code) {
        const lang = normalizeLanguageCode(code);
        if (!lang) return false;
        try {
            const resp = await fetch(`/assets/languages/${encodeURIComponent(lang)}.json`, { cache: 'no-store' });
            if (!resp.ok) return false;
            const data = await resp.json();
            COPY = (data && typeof data === 'object') ? data : {};
            currentLanguage = lang;
            return true;
        } catch (_e) {
            return false;
        }
    }

    function setTopbarHintText(el, text) {
        if (!el) return;
        el.textContent = String(text || '');
    }

    function renderUnattendedState(runtime) {
        const el = document.getElementById('unattended_state_text');
        if (!el) return;
        const unattendedMode = false; // Diff viewer is read-only; show "off".
        const mins = parseBoundedInt(globalOptions.stop_after_minutes, 0, 24 * 60, 0);
        const serverStartMs = Number(runtime?.last_start_unix || 0) * 1000;
        let stopText = txt('unattended_stop_disabled', '');
        if (mins > 0) {
            let remaining = mins;
            if (serverStartMs > 0) {
                const leftMs = Math.max(0, (serverStartMs + mins * 60 * 1000) - Date.now());
                remaining = Math.max(0, Math.ceil(leftMs / 60000));
            }
            stopText = fmt('unattended_stop_left_minutes', '', { minutes: remaining });
        }
        const resumeLeft = parseBoundedInt(globalOptions.auto_resume_attempts, 0, 20, 0);
        const line = fmt('unattended_state_line', '', {
            mode: unattendedMode ? txt('unattended_mode_on', '') : txt('unattended_mode_off', ''),
            resume_left: String(resumeLeft),
            stop: stopText,
        });
        setTopbarHintText(el, line);
    }

    function classifyDiffLine(line) {
        if (line.startsWith('+') && !line.startsWith('+++')) return 'diff-line-add';
        if (line.startsWith('-') && !line.startsWith('---')) return 'diff-line-del';
        return 'diff-line-other';
    }

    function renderDiff(diffText) {
        const el = document.getElementById('diff_view');
        if (!el) return;
        const safe = String(diffText || '');
        if (!safe.trim()) {
            el.innerHTML = `<div class="hint">${escapeHtml(txt('diff_empty', ''))}</div>`;
            return;
        }
        const lines = safe.split('\n');
        el.innerHTML = lines.map(line => {
            return `<div class="${classifyDiffLine(line)}">${escapeHtml(line)}</div>`;
        }).join('');
    }

    function escapeHtml(text) {
        return String(text || '')
            .replace(/&/g, '&amp;')
            .replace(/</g, '&lt;')
            .replace(/>/g, '&gt;');
    }

    function applyStaticCopy() {
        document.title = txt('diff_page_title', document.title);
        const brand = document.getElementById('topbar_brand');
        if (brand) brand.textContent = txt('app_brand', 'KACF');
        const back = document.getElementById('back_to_main_btn');
        if (back) back.textContent = txt('btn_back_to_main', '');
        const title = document.getElementById('diff_title');
        if (title) title.textContent = txt('diff_view_title', '');
        const sub = document.getElementById('diff_sub');
        if (sub) sub.textContent = txt('diff_view_sub', '');
    }

    async function fetchUiState() {
        try {
            const resp = await fetch('/ui_state', { cache: 'no-store' });
            if (!resp.ok) return null;
            return await resp.json();
        } catch (_e) {
            return null;
        }
    }

    async function fetchDiffData(bucket) {
        try {
            const resp = await fetch(`/diff_data?bucket=${encodeURIComponent(bucket)}`, { cache: 'no-store' });
            if (!resp.ok) return null;
            return await resp.json();
        } catch (_e) {
            return null;
        }
    }

    function renderTopbarFromBucketState(s) {
        const runbar = document.getElementById('runbar');
        if (runbar) runbar.dataset.state = String(s?.run_state || 'idle');
        const runText = fmt('runbar_line', '', { state: String(s?.run_text || txt('run_idle', '')) });
        setTopbarHintText(document.getElementById('runbar_text'), runText);
        setTopbarHintText(
            document.getElementById('running_project_text'),
            fmt('running_project_line', '', { name: String(s?.project_label || txt('running_project_none', '')) })
        );
    }

    function bindBackButton() {
        const btn = document.getElementById('back_to_main_btn');
        if (!btn) return;
        btn.addEventListener('click', () => {
            try {
                window.close();
                setTimeout(() => { window.location.href = '/'; }, 50);
            } catch (_e) {
                window.location.href = '/';
            }
        });
    }

    async function init() {
        const cache = await fetchUiCache();
        sharedConfig.language = normalizeLanguageCode(cache?.shared_config?.language || '');
        globalOptions.stop_after_minutes = String(cache?.global_options?.stop_after_minutes || '').trim();
        globalOptions.auto_resume_attempts = String(cache?.global_options?.auto_resume_attempts || '').trim();

        const available = await fetchLanguages();
        const lang = choosePreferredLanguage(available, sharedConfig.language);
        const ok = await loadCopyForLanguage(lang);
        if (!ok) {
            COPY = {};
        }
        applyStaticCopy();
        bindBackButton();

        const bucket = qs('bucket');
        if (!bucket) {
            renderDiff('');
            setTopbarHintText(document.getElementById('runbar_text'), fmt('runbar_line', '', { state: txt('run_idle', '') }));
            setTopbarHintText(document.getElementById('running_project_text'), fmt('running_project_line', '', { name: txt('running_project_none', '') }));
            renderUnattendedState(null);
            return;
        }

        const tick = async () => {
            const [uiState, diffData] = await Promise.all([fetchUiState(), fetchDiffData(bucket)]);
            if (diffData) {
                renderTopbarFromBucketState(diffData);
                renderDiff(diffData.diff_text || '');
            }
            renderUnattendedState(uiState?.runtime || null);
        };
        await tick();
        setInterval(tick, 1200);
    }

    init();
})();
