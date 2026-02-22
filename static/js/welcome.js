(() => {
    const README_URL = '/assets/welcome_intro.md';
    let COPY = {};

    function txt(k, fb) {
        const v = COPY && typeof COPY === 'object' ? COPY[k] : null;
        return typeof v === 'string' ? v : (fb || '');
    }

    function setText(id, value) {
        const el = document.getElementById(id);
        if (el) el.textContent = value;
    }

    function normalizeLanguageCode(code) {
        const s = String(code || '').trim();
        if (!s) return '';
        if (s.toLowerCase() === 'zh-cn') return 'zh-CN';
        if (s.toLowerCase() === 'en') return 'en';
        return s;
    }

    async function fetchLanguages() {
        const resp = await fetch('/assets/languages/list', { cache: 'no-store' });
        if (!resp.ok) throw new Error(`languages failed: ${resp.status}`);
        const data = await resp.json();
        const langs = Array.isArray(data.languages) ? data.languages : [];
        return langs.map(normalizeLanguageCode).filter(Boolean);
    }

    function choosePreferredLanguage(langs) {
        const items = Array.isArray(langs) ? langs : [];
        if (!items.length) return '';
        if (items.length === 1) return items[0];
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

    async function loadCopy(lang) {
        const code = normalizeLanguageCode(lang);
        const resp = await fetch(`/assets/languages/${encodeURIComponent(code)}.json`, { cache: 'no-store' });
        if (!resp.ok) return false;
        const data = await resp.json();
        if (!data || typeof data !== 'object') return false;
        COPY = data;
        document.documentElement.lang = code || 'en';
        return true;
    }

    async function initCopyAndUi() {
        let langs = ['en'];
        try {
            const fetched = await fetchLanguages();
            if (fetched.length) langs = fetched;
        } catch (_e) {}
        const picked = choosePreferredLanguage(langs);
        await loadCopy(picked || langs[0] || 'en');

        document.title = txt('welcome_page_title', document.title);
        setText('welcome_kicker', txt('welcome_kicker', 'KINOGRAM'));
        setText('welcome_title', txt('welcome_title', 'KACF'));
        setText('welcome_subtitle', txt('welcome_subtitle', 'Describe your idea in one sentence, and KACF builds runnable software automatically.'));
        setText('enter_program_btn', txt('welcome_enter_program', 'Enter Program'));
        setText('open_github_btn', txt('welcome_open_github', 'Open GitHub Repository'));
        setText('welcome_intro_title', txt('welcome_intro_title', 'Project Introduction'));
        setText('welcome_readme', txt('welcome_intro_loading', 'Loading local introduction...'));
    }

    function escapeHtml(text) {
        return String(text || '')
            .replace(/&/g, '&amp;')
            .replace(/</g, '&lt;')
            .replace(/>/g, '&gt;');
    }

    function markdownToHtml(md) {
        const src = String(md || '');
        const lines = src.split(/\r?\n/).slice(0, 220);
        const out = [];
        let inList = false;
        let inCode = false;
        for (const raw of lines) {
            const line = raw || '';
            if (line.trim().startsWith('```')) {
                if (!inCode) {
                    if (inList) {
                        out.push('</ul>');
                        inList = false;
                    }
                    out.push('<pre><code>');
                    inCode = true;
                } else {
                    out.push('</code></pre>');
                    inCode = false;
                }
                continue;
            }
            if (inCode) {
                out.push(`${escapeHtml(line)}\n`);
                continue;
            }
            if (!line.trim()) {
                if (inList) {
                    out.push('</ul>');
                    inList = false;
                }
                continue;
            }
            if (line.startsWith('# ')) {
                if (inList) {
                    out.push('</ul>');
                    inList = false;
                }
                out.push(`<h3>${escapeHtml(line.slice(2).trim())}</h3>`);
                continue;
            }
            if (line.startsWith('## ')) {
                if (inList) {
                    out.push('</ul>');
                    inList = false;
                }
                out.push(`<h4>${escapeHtml(line.slice(3).trim())}</h4>`);
                continue;
            }
            if (line.startsWith('- ') || line.startsWith('* ')) {
                if (!inList) {
                    out.push('<ul>');
                    inList = true;
                }
                out.push(`<li>${escapeHtml(line.slice(2).trim())}</li>`);
                continue;
            }
            if (inList) {
                out.push('</ul>');
                inList = false;
            }
            out.push(`<p>${escapeHtml(line.trim())}</p>`);
        }
        if (inList) out.push('</ul>');
        if (inCode) out.push('</code></pre>');
        return out.join('');
    }

    async function refreshEnterButton() {
        const btn = document.getElementById('enter_program_btn');
        if (!btn) return;
        try {
            const resp = await fetch('/auth/me', { cache: 'no-store' });
            if (!resp.ok) return;
            const data = await resp.json();
            if (data && data.logged_in) {
                btn.href = '/';
                btn.textContent = txt('welcome_enter_workspace', 'Enter Workspace');
            }
        } catch (_e) {}
    }

    async function loadReadme() {
        const root = document.getElementById('welcome_readme');
        if (!root) return;
        try {
            const resp = await fetch(README_URL, { cache: 'no-store' });
            if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
            const text = await resp.text();
            root.innerHTML = markdownToHtml(text);
        } catch (_e) {
            root.innerHTML = `<p>${escapeHtml(txt('welcome_intro_load_failed', 'Failed to load local introduction. Please contact the administrator.'))}</p>`;
        }
    }

    (async () => {
        await initCopyAndUi();
        await Promise.all([refreshEnterButton(), loadReadme()]);
    })();
})();
