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
        let data = null;
        try { data = JSON.parse(text); } catch (_e) {}
        return { ok: resp.ok, status: resp.status, text, data };
    }

    function readVal(id) {
        const el = document.getElementById(id);
        return el ? String(el.value || '').trim() : '';
    }

    async function main() {
        await loadLang();
        document.title = txt('account_page_title', document.title);

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

        setText('account_security_title', txt('account_security_title', 'Security'));
        setText('account_security_sub', txt('account_security_sub', 'Update username/email. Changes require verifying your current login method.'));
        setText('label_acct_new_username', txt('label_acct_new_username', 'New username'));
        setText('acct_change_username_btn', txt('acct_change_username_btn', 'Change username'));
        setText('label_acct_new_email', txt('label_acct_new_email', 'New email'));
        setText('label_acct_new_email_code', txt('label_acct_new_email_code', 'New email code'));
        setText('label_acct_current_email_code', txt('label_acct_current_email_code', 'Current email code'));
        setText('acct_request_current_email_code_btn', txt('acct_request_current_email_code_btn', 'Request current email code'));
        setText('acct_request_new_email_code_btn', txt('acct_request_new_email_code_btn', 'Request new email code'));
        setText('acct_confirm_new_email_btn', txt('acct_confirm_new_email_btn', 'Confirm email change'));

        setText('account_login_option_title', txt('account_login_option_title', 'Login options'));
        setText('account_login_option_sub', txt('account_login_option_sub', 'Switch login method. If you choose a no-password option, your stored password will be removed.'));
        setText('label_acct_login_option', txt('label_acct_login_option', 'Login option'));
        setText('label_acct_new_password_for_option', txt('label_acct_new_password_for_option', 'Password (if required)'));
        setText('acct_change_login_option_btn', txt('acct_change_login_option_btn', 'Save login option'));
        const optPw = document.querySelector('#acct_login_option option[value="password_only"]');
        const optPwEmail = document.querySelector('#acct_login_option option[value="password_email_2fa"]');
        const optEmail = document.querySelector('#acct_login_option option[value="email_only"]');
        if (optPw) optPw.textContent = txt('login_option_password_only', 'Password only');
        if (optPwEmail) optPwEmail.textContent = txt('login_option_password_email_2fa', 'Password + email code');
        if (optEmail) optEmail.textContent = txt('login_option_email_only', 'Email code only (no password)');

        const meResp = await getJson('/account/api/self');
        if (!meResp.ok || !meResp.data) {
            location.href = '/login';
            return;
        }
        const me = meResp.data;

        document.getElementById('acct_username').value = me.username || '';
        document.getElementById('acct_email').value = me.email || '';
        document.getElementById('acct_nickname').value = me.nickname || '';
        document.getElementById('acct_new_username').value = '';
        document.getElementById('acct_new_email').value = '';
        document.getElementById('acct_new_email_code').value = '';
        document.getElementById('acct_current_email_code').value = '';
        document.getElementById('acct_login_option').value = me.login_option || 'password_only';

        function currentLoginOption() {
            return String(document.getElementById('acct_login_option').value || 'password_only');
        }

        function requiresPassword(option) {
            return option === 'password_only' || option === 'password_email_2fa';
        }
        function requiresEmailCode(option) {
            return option === 'email_only' || option === 'password_email_2fa';
        }

        function applySecurityUiByOption(opt) {
            const pwRow = document.getElementById('acct_old_password').closest('.row');
            const pwTitle = document.getElementById('account_password_title');
            const pwSub = document.getElementById('account_password_sub');
            const pwBtn = document.getElementById('acct_change_password_btn');
            const pwHint = document.getElementById('acct_password_hint');
            const oldPw = document.getElementById('acct_old_password');
            const newPw = document.getElementById('acct_new_password');

            const showPw = requiresPassword(opt);
            if (pwRow) pwRow.style.display = showPw ? '' : 'none';
            if (pwTitle) pwTitle.style.display = showPw ? '' : 'none';
            if (pwSub) pwSub.style.display = showPw ? '' : 'none';
            if (pwBtn) pwBtn.style.display = showPw ? '' : 'none';
            if (pwHint && !showPw) pwHint.textContent = txt('password_hidden_no_password', 'Password is disabled for this login option.');
            if (!showPw) {
                oldPw.value = '';
                newPw.value = '';
            }

            const emailCodeNeeded = requiresEmailCode(me.login_option || 'password_only');
            document.getElementById('acct_request_current_email_code_btn').style.display = emailCodeNeeded ? '' : 'none';
            const curCode = document.getElementById('acct_current_email_code');
            if (curCode) curCode.placeholder = emailCodeNeeded ? txt('ph_email_code_required', '') : txt('ph_email_code_optional', '');

            const needsPwForOption = requiresPassword(opt) && !requiresPassword(me.login_option || 'password_only');
            const np = document.getElementById('acct_new_password_for_option');
            if (np) {
                np.parentElement.style.display = needsPwForOption ? '' : 'none';
                if (!needsPwForOption) np.value = '';
            }
        }
        applySecurityUiByOption(currentLoginOption());
        document.getElementById('acct_login_option').addEventListener('change', () => {
            applySecurityUiByOption(currentLoginOption());
        });

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
                current_email_code: readVal('acct_current_email_code'),
            });
            if (!r.ok) {
                showHint('acct_password_hint', r.text || `HTTP ${r.status}`, true);
            } else {
                document.getElementById('acct_old_password').value = '';
                document.getElementById('acct_new_password').value = '';
                showHint('acct_password_hint', txt('status_saved', 'Saved.'), false);
            }
        });

        document.getElementById('acct_request_current_email_code_btn').addEventListener('click', async () => {
            showHint('acct_email_hint', txt('auth_working', 'Working...'), false);
            const r = await postJson('/account/api/request_current_email_code', {});
            if (!r.ok) {
                showHint('acct_email_hint', r.text || `HTTP ${r.status}`, true);
                return;
            }
            const code = r.data && r.data.dev_code ? String(r.data.dev_code) : '';
            showHint('acct_email_hint', code ? `${txt('dev_code_hint', 'Dev code')}: ${code}` : txt('code_sent', 'Code issued.'), false);
        });

        document.getElementById('acct_change_username_btn').addEventListener('click', async () => {
            showHint('acct_username_hint', txt('auth_working', 'Working...'), false);
            const r = await postJson('/account/api/change_username', {
                new_username: readVal('acct_new_username'),
                old_password: readVal('acct_old_password'),
                current_email_code: readVal('acct_current_email_code'),
            });
            if (!r.ok) {
                showHint('acct_username_hint', r.text || `HTTP ${r.status}`, true);
                return;
            }
            showHint('acct_username_hint', txt('status_saved', 'Saved.'), false);
            // Refresh displayed username.
            location.reload();
        });

        document.getElementById('acct_request_new_email_code_btn').addEventListener('click', async () => {
            showHint('acct_email_hint', txt('auth_working', 'Working...'), false);
            const r = await postJson('/account/api/request_new_email_code', { new_email: readVal('acct_new_email') });
            if (!r.ok) {
                showHint('acct_email_hint', r.text || `HTTP ${r.status}`, true);
                return;
            }
            const code = r.data && r.data.dev_code ? String(r.data.dev_code) : '';
            showHint('acct_email_hint', code ? `${txt('dev_code_hint', 'Dev code')}: ${code}` : txt('code_sent', 'Code issued.'), false);
        });

        document.getElementById('acct_confirm_new_email_btn').addEventListener('click', async () => {
            showHint('acct_email_hint', txt('auth_working', 'Working...'), false);
            const r = await postJson('/account/api/confirm_email_change', {
                new_email: readVal('acct_new_email'),
                new_email_code: readVal('acct_new_email_code'),
                old_password: readVal('acct_old_password'),
                current_email_code: readVal('acct_current_email_code'),
            });
            if (!r.ok) {
                showHint('acct_email_hint', r.text || `HTTP ${r.status}`, true);
                return;
            }
            showHint('acct_email_hint', txt('status_saved', 'Saved.'), false);
            location.reload();
        });

        document.getElementById('acct_change_login_option_btn').addEventListener('click', async () => {
            showHint('acct_login_option_hint', txt('auth_working', 'Working...'), false);
            const r = await postJson('/account/api/change_login_option', {
                new_option: currentLoginOption(),
                old_password: readVal('acct_old_password'),
                current_email_code: readVal('acct_current_email_code'),
                new_password: readVal('acct_new_password_for_option'),
            });
            if (!r.ok) {
                showHint('acct_login_option_hint', r.text || `HTTP ${r.status}`, true);
                return;
            }
            showHint('acct_login_option_hint', txt('status_saved', 'Saved.'), false);
            location.reload();
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
