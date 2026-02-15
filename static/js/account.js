(function () {
    'use strict';

    let COPY = {};
    function txt(k, fb) {
        const v = COPY && typeof COPY === 'object' ? COPY[k] : null;
        return typeof v === 'string' ? v : (fb || '');
    }

    async function loadLang() {
        const respList = await fetch('/assets/languages/list', { cache: 'no-store' });
        const list = respList.ok ? (await respList.json()) : { languages: ['en'] };
        const langs = Array.isArray(list.languages) ? list.languages : ['en'];
        const picked = (navigator.language || '').toLowerCase().startsWith('zh') && langs.includes('zh-CN') ? 'zh-CN' : (langs.includes('en') ? 'en' : langs[0]);
        const resp = await fetch(`/assets/languages/${encodeURIComponent(picked)}.json`, { cache: 'no-store' });
        COPY = resp.ok ? (await resp.json()) : {};
        document.documentElement.lang = picked;
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

    async function getJson(url) {
        const resp = await fetch(url, { cache: 'no-store' });
        const text = await resp.text();
        let data = null;
        try { data = JSON.parse(text); } catch (_e) {}
        return { ok: resp.ok, status: resp.status, text, data };
    }

    async function postJson(url, body) {
        const resp = await fetch(url, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body || {}) });
        const text = await resp.text();
        return { ok: resp.ok, status: resp.status, text };
    }

    function readVal(id) {
        const el = document.getElementById(id);
        return el ? String(el.value || '').trim() : '';
    }

    async function main() {
        await loadLang();

        setText('account_back_btn', txt('btn_back', 'Back'));
        setText('account_brand', txt('app_name', 'KACF'));
        setText('account_logout_btn', txt('btn_logout', 'Logout'));
        setText('account_title', txt('account_title', 'Manage Your KACF Account'));
        setText('account_sub', txt('account_sub', ''));
        setText('account_profile_title', txt('account_profile_title', 'Profile'));
        setText('label_acct_username', txt('label_acct_username', 'Username'));
        setText('label_acct_email', txt('label_acct_email', 'Email'));
        setText('label_acct_nickname', txt('label_acct_nickname', 'Nickname'));
        setText('acct_save_profile_btn', txt('btn_save', 'Save'));
        setText('account_password_title', txt('account_password_title', 'Password'));
        setText('account_password_sub', txt('account_password_sub', ''));
        setText('label_acct_old_password', txt('label_acct_old_password', 'Current password'));
        setText('label_acct_new_password', txt('label_acct_new_password', 'New password'));
        setText('acct_change_password_btn', txt('btn_change_password', 'Change password'));
        setText('acct_recover_btn', txt('btn_recover_account', 'Recover account'));

        const meResp = await getJson('/auth/me');
        if (!meResp.ok || !meResp.data || !meResp.data.logged_in || meResp.data.guest) {
            location.href = '/login';
            return;
        }
        const me = meResp.data;

        document.getElementById('acct_username').value = me.username || '';
        document.getElementById('acct_email').value = me.email || '';
        document.getElementById('acct_nickname').value = me.nickname || '';

        document.getElementById('account_logout_btn').addEventListener('click', async () => {
            await postJson('/auth/logout', {});
            location.href = '/login';
        });

        document.getElementById('acct_save_profile_btn').addEventListener('click', async () => {
            showHint('acct_profile_hint', txt('auth_working', 'Working...'), false);
            const r = await postJson('/account/api/profile', { nickname: readVal('acct_nickname') });
            if (!r.ok) {
                showHint('acct_profile_hint', r.text || `HTTP ${r.status}`, true);
            } else {
                showHint('acct_profile_hint', txt('status_saved', 'Saved.'), false);
            }
        });

        document.getElementById('acct_change_password_btn').addEventListener('click', async () => {
            showHint('acct_password_hint', txt('auth_working', 'Working...'), false);
            const r = await postJson('/account/api/password', {
                old_password: readVal('acct_old_password'),
                new_password: readVal('acct_new_password'),
            });
            if (!r.ok) {
                showHint('acct_password_hint', r.text || `HTTP ${r.status}`, true);
            } else {
                document.getElementById('acct_old_password').value = '';
                document.getElementById('acct_new_password').value = '';
                showHint('acct_password_hint', txt('status_saved', 'Saved.'), false);
            }
        });

        document.getElementById('acct_recover_btn').addEventListener('click', (e) => {
            e.preventDefault();
            alert(txt('account_recover_hint', 'If you cannot verify, contact the administrator.'));
        });
    }

    main().catch(e => {
        console.error(e);
        showHint('acct_profile_hint', String(e && e.message ? e.message : e), true);
    });
})();

