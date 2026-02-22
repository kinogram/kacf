(function () {
    'use strict';

    let COPY = {};
    function txt(k, fb) {
        const v = COPY && typeof COPY === 'object' ? COPY[k] : null;
        return typeof v === 'string' ? v : (fb || '');
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

    function setText(id, value) {
        const el = document.getElementById(id);
        if (el) el.textContent = value;
    }

    function showHint(id, msg, isErr) {
        const el = document.getElementById(id);
        if (!el) return;
        el.textContent = msg || '';
        el.style.color = isErr ? '#8a1f17' : '';
    }

    async function postJson(url, body) {
        const resp = await fetch(url, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body || {}),
        });
        const text = await resp.text();
        return { ok: resp.ok, status: resp.status, text };
    }

    function readValue(id) {
        const el = document.getElementById(id);
        return el ? String(el.value || '').trim() : '';
    }

    function setTab(active) {
        const loginTab = document.getElementById('tab_login');
        const regTab = document.getElementById('tab_register');
        const loginBlock = document.getElementById('login_block');
        const regBlock = document.getElementById('register_block');
        if (loginTab) loginTab.classList.toggle('active', active === 'login');
        if (regTab) regTab.classList.toggle('active', active === 'register');
        if (loginBlock) loginBlock.style.display = active === 'login' ? '' : 'none';
        if (regBlock) regBlock.style.display = active === 'register' ? '' : 'none';
        showHint('login_hint', '', false);
        showHint('register_hint', '', false);
        const codeBlock = document.getElementById('login_code_block');
        const verifyBtn = document.getElementById('login_verify_btn');
        if (codeBlock) codeBlock.style.display = 'none';
        if (verifyBtn) verifyBtn.style.display = 'none';
    }

    async function initCopyAndUi() {
        const langs = await fetchLanguages();
        const picked = choosePreferredLanguage(langs);
        await loadCopy(picked || langs[0] || 'en');

        setText('auth_brand', txt('app_name', 'KACF'));
        setText('auth_welcome_title', txt('auth_welcome_title', 'Welcome'));
        setText('auth_welcome_sub', txt('auth_welcome_sub', ''));
        setText('tab_login', txt('auth_tab_login', 'Login'));
        setText('tab_register', txt('auth_tab_register', 'Register'));

        setText('label_login_identifier', txt('label_login_identifier', 'Username or Email'));
        setText('label_login_password', txt('label_login_password', 'Password'));
        setText('label_login_code', txt('label_login_code', 'Email code'));
        setText('login_btn', txt('btn_login', 'Login'));
        setText('login_verify_btn', txt('btn_verify_code', 'Verify code'));
        setText('guest_btn', txt('btn_guest', 'Continue as guest'));

        setText('label_reg_username', txt('label_reg_username', 'Username'));
        setText('label_reg_nickname', txt('label_reg_nickname', 'Nickname'));
        setText('label_reg_email', txt('label_reg_email', 'Email'));
        setText('label_reg_password', txt('label_reg_password', 'Password'));
        setText('register_btn', txt('btn_register', 'Register'));

        setText('bootstrap_title', txt('bootstrap_title', 'Create Administrator'));
        setText('bootstrap_sub', txt('bootstrap_sub', ''));
        setText('label_boot_username', txt('label_boot_username', 'Admin username'));
        setText('label_boot_nickname', txt('label_boot_nickname', 'Admin nickname'));
        setText('label_boot_email', txt('label_boot_email', 'Admin email'));
        setText('label_boot_password', txt('label_boot_password', 'Admin password'));
        setText('bootstrap_btn', txt('btn_bootstrap_admin', 'Create admin'));

        const gearHint = document.getElementById('auth_lang_hint');
        if (gearHint) gearHint.textContent = txt('auth_lang_hint', '');
    }

    async function checkBootstrap() {
        const resp = await fetch('/auth/bootstrap_status', { cache: 'no-store' });
        if (!resp.ok) return;
        const data = await resp.json();
        const hasAny = !!data.has_any_user;
        const block = document.getElementById('bootstrap_admin_block');
        if (block) block.style.display = hasAny ? 'none' : '';
    }

    function wireEvents() {
        const loginTab = document.getElementById('tab_login');
        const regTab = document.getElementById('tab_register');
        if (loginTab) loginTab.addEventListener('click', () => setTab('login'));
        if (regTab) regTab.addEventListener('click', () => setTab('register'));

        const loginBtn = document.getElementById('login_btn');
        if (loginBtn) loginBtn.addEventListener('click', async () => {
            showHint('login_hint', txt('auth_working', 'Working...'), false);
            const r = await postJson('/auth/login', {
                identifier: readValue('login_identifier'),
                password: readValue('login_password'),
            });
            if (r.ok) {
                location.href = '/';
            } else if (r.status === 409) {
                let data = null;
                try { data = JSON.parse(r.text); } catch (_e) {}
                if (data && data.need === 'email_code') {
                    const codeBlock = document.getElementById('login_code_block');
                    const verifyBtn = document.getElementById('login_verify_btn');
                    if (codeBlock) codeBlock.style.display = '';
                    if (verifyBtn) verifyBtn.style.display = '';
                    const hint = document.getElementById('login_code_hint');
                    if (hint) hint.textContent = (data.dev_code ? `${txt('dev_code_hint', 'Dev code')}: ${data.dev_code}` : '');
                    const retry = Number(data.retry_after_secs || 0);
                    const extra = retry > 0 ? ` (${retry}s)` : '';
                    showHint('login_hint', `${txt('login_need_code', 'Please enter the email code.')}${extra}`, false);
                } else {
                    showHint('login_hint', r.text || `HTTP ${r.status}`, true);
                }
            } else {
                showHint('login_hint', r.text || `HTTP ${r.status}`, true);
            }
        });

        const verifyBtn = document.getElementById('login_verify_btn');
        if (verifyBtn) verifyBtn.addEventListener('click', async () => {
            showHint('login_hint', txt('auth_working', 'Working...'), false);
            const r = await postJson('/auth/verify_email_code', {
                identifier: readValue('login_identifier'),
                code: readValue('login_code'),
            });
            if (r.ok) {
                location.href = '/';
            } else {
                showHint('login_hint', r.text || `HTTP ${r.status}`, true);
            }
        });

        const guestBtn = document.getElementById('guest_btn');
        if (guestBtn) guestBtn.addEventListener('click', async () => {
            showHint('login_hint', txt('auth_working', 'Working...'), false);
            const r = await postJson('/auth/guest', {});
            if (r.ok) {
                location.href = '/';
            } else {
                showHint('login_hint', r.text || `HTTP ${r.status}`, true);
            }
        });

        const regBtn = document.getElementById('register_btn');
        if (regBtn) regBtn.addEventListener('click', async () => {
            showHint('register_hint', txt('auth_working', 'Working...'), false);
            const r = await postJson('/auth/register', {
                username: readValue('reg_username'),
                nickname: readValue('reg_nickname'),
                email: readValue('reg_email'),
                password: readValue('reg_password'),
            });
            if (r.ok) {
                showHint('register_hint', txt('auth_register_ok', 'Registered. Please login.'), false);
                setTab('login');
            } else {
                showHint('register_hint', r.text || `HTTP ${r.status}`, true);
            }
        });

        const bootBtn = document.getElementById('bootstrap_btn');
        if (bootBtn) bootBtn.addEventListener('click', async () => {
            showHint('bootstrap_hint', txt('auth_working', 'Working...'), false);
            const r = await postJson('/auth/bootstrap_admin', {
                username: readValue('boot_username'),
                nickname: readValue('boot_nickname'),
                email: readValue('boot_email'),
                password: readValue('boot_password'),
            });
            if (r.ok) {
                showHint('bootstrap_hint', txt('bootstrap_ok', 'Admin created. Please login.'), false);
                await checkBootstrap();
            } else {
                showHint('bootstrap_hint', r.text || `HTTP ${r.status}`, true);
            }
        });
    }

    (async function main() {
        try {
            await initCopyAndUi();
            await checkBootstrap();
            wireEvents();
            setTab('login');
        } catch (e) {
            console.error(e);
            showHint('login_hint', String(e && e.message ? e.message : e), true);
        }
    })();
})();
