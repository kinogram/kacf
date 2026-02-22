(function () {
    'use strict';

    let COPY = {};
    let settingsDirty = false;
    let suppressDirtyTracking = false;
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

    function setPlaceholder(id, value) {
        const el = document.getElementById(id);
        if (el) el.setAttribute('placeholder', value || '');
    }

    function showHint(id, msg, isErr) {
        const el = document.getElementById(id);
        if (!el) return;
        el.textContent = msg || '';
        el.style.color = isErr ? '#8a1f17' : '';
    }

    function setSettingsDirty(v) {
        settingsDirty = !!v;
    }

    function bindDirtyTracking() {
        const ids = [
            'admin_toggle_login',
            'admin_toggle_registration',
            'admin_toggle_guest',
            'admin_toggle_non_admin_login',
            'admin_toggle_email_dev_mode',
            'admin_email_ttl_secs',
            'admin_email_cooldown_secs',
            'admin_email_issue_per_min',
            'admin_email_verify_per_min',
            'admin_toggle_email_domain_allowlist_enabled',
            'admin_email_domain_allowlist',
            'admin_toggle_smtp_enabled',
            'admin_toggle_smtp_starttls',
            'admin_smtp_host',
            'admin_smtp_port',
            'admin_smtp_username',
            'admin_smtp_password',
            'admin_smtp_from',
        ];
        ids.forEach((id) => {
            const el = document.getElementById(id);
            if (!el) return;
            const onMutate = () => {
                if (suppressDirtyTracking) return;
                setSettingsDirty(true);
            };
            el.addEventListener('input', onMutate);
            el.addEventListener('change', onMutate);
        });
    }

    function bindLeaveGuard() {
        window.addEventListener('beforeunload', (e) => {
            if (!settingsDirty) return;
            e.preventDefault();
            e.returnValue = '';
        });
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

    async function del(url) {
        const resp = await fetch(url, { method: 'DELETE' });
        const text = await resp.text();
        return { ok: resp.ok, status: resp.status, text };
    }

    function readVal(id) {
        const el = document.getElementById(id);
        return el ? String(el.value || '').trim() : '';
    }

    function rowBtn(label, onClick, cls) {
        const b = document.createElement('button');
        b.className = `btn-ghost ${cls || ''}`.trim();
        b.type = 'button';
        b.textContent = label;
        b.addEventListener('click', onClick);
        return b;
    }

    function renderUsers(users) {
        const table = document.getElementById('admin_users_table');
        if (!table) return;
        const tbody = table.querySelector('tbody');
        if (!tbody) return;
        while (tbody.firstChild) tbody.removeChild(tbody.firstChild);

        (users || []).forEach(u => {
            const tr = document.createElement('tr');
            const tdUser = document.createElement('td');
            tdUser.textContent = `${u.nickname || u.username} (${u.username})`;
            const tdRole = document.createElement('td');
            tdRole.textContent = String(u.role || '');
            const tdEmail = document.createElement('td');
            tdEmail.textContent = String(u.email || '');
            const tdPw = document.createElement('td');
            tdPw.textContent = (u.password == null ? '' : String(u.password));
            const tdBanned = document.createElement('td');
            tdBanned.textContent = u.banned ? 'YES' : 'NO';
            const tdAct = document.createElement('td');
            tdAct.style.display = 'flex';
            tdAct.style.gap = '8px';
            tdAct.style.flexWrap = 'wrap';

            const banLabel = u.banned ? txt('admin_unban', 'Unban') : txt('admin_ban', 'Ban');
            tdAct.appendChild(rowBtn(banLabel, async () => {
                const ok = confirm(`${banLabel} ${u.username}?`);
                if (!ok) return;
                const r = await postJson(`/admin/api/users/${encodeURIComponent(u.username)}/ban`, { banned: !u.banned });
                if (!r.ok) {
                    alert(r.text || `HTTP ${r.status}`);
                    return;
                }
                await refreshAll();
            }));
            tdAct.appendChild(rowBtn(txt('admin_notice', 'Notice'), async () => {
                const msg = prompt(txt('admin_notice_prompt', 'Forced notice message (empty to clear):'), '');
                if (msg === null) return;
                let secs = 0;
                if (msg.trim()) {
                    const raw = prompt(txt('admin_notice_secs_prompt', 'Min seconds before closing (0-300):'), '5');
                    if (raw === null) return;
                    secs = Math.max(0, Math.min(300, parseInt(raw, 10) || 0));
                }
                const r = await postJson(`/admin/api/users/${encodeURIComponent(u.username)}/notice`, {
                    message: msg,
                    min_seconds: secs,
                });
                if (!r.ok) {
                    alert(r.text || `HTTP ${r.status}`);
                    return;
                }
                await refreshAll();
            }));
            tdAct.appendChild(rowBtn(txt('admin_set_password', 'Set password'), async () => {
                const pw = prompt(txt('admin_set_password_prompt', 'New password (empty to clear):'), '');
                if (pw === null) return;
                const r = await postJson(`/admin/api/users/${encodeURIComponent(u.username)}/password`, { password: pw });
                if (!r.ok) {
                    alert(r.text || `HTTP ${r.status}`);
                    return;
                }
                await refreshAll();
            }));
            tdAct.appendChild(rowBtn(txt('admin_delete', 'Delete'), async () => {
                const ok = confirm(`${txt('admin_delete', 'Delete')} ${u.username}?`);
                if (!ok) return;
                const r = await del(`/admin/api/users/${encodeURIComponent(u.username)}`);
                if (!r.ok) {
                    alert(r.text || `HTTP ${r.status}`);
                    return;
                }
                await refreshAll();
            }, 'btn-danger'));

            tr.appendChild(tdUser);
            tr.appendChild(tdRole);
            tr.appendChild(tdEmail);
            tr.appendChild(tdPw);
            tr.appendChild(tdBanned);
            tr.appendChild(tdAct);
            tbody.appendChild(tr);
        });
    }

    async function refreshAll() {
        const s = await getJson('/admin/api/settings');
        suppressDirtyTracking = true;
        if (s.ok && s.data) {
            document.getElementById('admin_toggle_login').checked = !!s.data.login_enabled;
            document.getElementById('admin_toggle_registration').checked = !!s.data.registration_enabled;
            document.getElementById('admin_toggle_guest').checked = !!s.data.guest_enabled;
            document.getElementById('admin_toggle_non_admin_login').checked = !!s.data.non_admin_login_enabled;
            document.getElementById('admin_toggle_email_dev_mode').checked = !!s.data.email_code_dev_mode;
            document.getElementById('admin_email_ttl_secs').value = String(s.data.email_code_ttl_secs || 600);
            document.getElementById('admin_email_cooldown_secs').value = String(s.data.email_code_resend_cooldown_secs || 60);
            document.getElementById('admin_email_issue_per_min').value = String(s.data.email_issue_per_min || 3);
            document.getElementById('admin_email_verify_per_min').value = String(s.data.email_verify_per_min || 12);
            document.getElementById('admin_toggle_email_domain_allowlist_enabled').checked = s.data.email_domain_allowlist_enabled !== false;
            document.getElementById('admin_email_domain_allowlist').value = Array.isArray(s.data.email_domain_allowlist)
                ? s.data.email_domain_allowlist.join('\n')
                : '';
            document.getElementById('admin_toggle_smtp_enabled').checked = !!s.data.smtp_enabled;
            document.getElementById('admin_toggle_smtp_starttls').checked = !!s.data.smtp_starttls;
            document.getElementById('admin_smtp_host').value = String(s.data.smtp_host || '');
            document.getElementById('admin_smtp_port').value = String(s.data.smtp_port || 587);
            document.getElementById('admin_smtp_username').value = String(s.data.smtp_username || '');
            document.getElementById('admin_smtp_password').value = String(s.data.smtp_password || '');
            document.getElementById('admin_smtp_from').value = String(s.data.smtp_from || 'noreply@localhost');
        }
        suppressDirtyTracking = false;
        setSettingsDirty(false);
        const u = await getJson('/admin/api/users');
        if (u.ok && Array.isArray(u.data)) {
            renderUsers(u.data);
        } else if (!u.ok) {
            showHint('admin_settings_hint', u.text || `HTTP ${u.status}`, true);
        }

        const a = await getJson('/admin/api/audit?tail=200');
        const pre = document.getElementById('admin_audit_log');
        if (pre) {
            if (a.ok && Array.isArray(a.data)) {
                pre.textContent = a.data.map(x => JSON.stringify(x)).join('\n');
            } else {
                pre.textContent = a.text || '';
            }
        }
    }

    async function main() {
        await loadLang();
        document.title = txt('admin_page_title', document.title);
        setText('admin_back_btn', txt('btn_back', 'Back'));
        setText('admin_brand', txt('app_name', 'KACF'));
        setText('admin_logout_btn', txt('btn_logout', 'Logout'));
        setText('admin_title', txt('admin_title', 'Administrator Panel'));
        setText('admin_sub', txt('admin_sub', ''));
        setText('admin_settings_title', txt('admin_settings_title', 'System switches'));
        setText('label_admin_toggle_login', txt('admin_toggle_login', 'Enable login'));
        setText('label_admin_toggle_registration', txt('admin_toggle_registration', 'Enable registration'));
        setText('label_admin_toggle_guest', txt('admin_toggle_guest', 'Enable guest mode'));
        setText('label_admin_toggle_non_admin_login', txt('admin_toggle_non_admin_login', 'Enable non-admin login'));
        setText('admin_save_settings_btn', txt('btn_save', 'Save'));
        setText('admin_email_settings_title', txt('admin_email_settings_title', 'Email verification'));
        setText('label_admin_toggle_email_dev_mode', txt('label_admin_toggle_email_dev_mode', 'Show dev code in response (dev only)'));
        setText('label_admin_email_ttl_secs', txt('label_admin_email_ttl_secs', 'Code TTL (secs)'));
        setText('label_admin_email_cooldown_secs', txt('label_admin_email_cooldown_secs', 'Resend cooldown (secs)'));
        setText('label_admin_email_issue_per_min', txt('label_admin_email_issue_per_min', 'Issue limit per minute'));
        setText('label_admin_email_verify_per_min', txt('label_admin_email_verify_per_min', 'Verify limit per minute'));
        setText('label_admin_toggle_email_domain_allowlist_enabled', txt('label_admin_toggle_email_domain_allowlist_enabled', 'Enable email domain allowlist'));
        setText('label_admin_email_domain_allowlist', txt('label_admin_email_domain_allowlist', 'Allowed email domains (one per line)'));
        setPlaceholder('admin_email_domain_allowlist', txt('ph_admin_email_domain_allowlist', ''));
        setText('admin_smtp_title', txt('admin_smtp_title', 'SMTP'));
        setText('label_admin_toggle_smtp_enabled', txt('label_admin_toggle_smtp_enabled', 'Enable SMTP send'));
        setText('label_admin_toggle_smtp_starttls', txt('label_admin_toggle_smtp_starttls', 'Use STARTTLS'));
        setText('label_admin_smtp_host', txt('label_admin_smtp_host', 'SMTP host'));
        setText('label_admin_smtp_port', txt('label_admin_smtp_port', 'SMTP port'));
        setText('label_admin_smtp_username', txt('label_admin_smtp_username', 'SMTP username'));
        setText('label_admin_smtp_password', txt('label_admin_smtp_password', 'SMTP password'));
        setText('label_admin_smtp_from', txt('label_admin_smtp_from', 'From email'));
        setPlaceholder('admin_smtp_host', txt('ph_admin_smtp_host', ''));
        setPlaceholder('admin_smtp_from', txt('ph_admin_smtp_from', ''));
        setText('admin_users_title', txt('admin_users_title', 'Users'));
        setText('label_admin_new_username', txt('label_admin_new_username', 'Username'));
        setText('label_admin_new_email', txt('label_admin_new_email', 'Email'));
        setText('label_admin_new_password', txt('label_admin_new_password', 'Password (plain)'));
        setText('label_admin_new_nickname', txt('label_admin_new_nickname', 'Nickname'));
        setText('label_admin_new_role', txt('label_admin_new_role', 'Role'));
        setText('admin_role_user_option', txt('admin_role_user', 'User'));
        setText('admin_role_admin_option', txt('admin_role_admin', 'Admin'));
        setText('admin_create_user_btn', txt('admin_create_user_btn', 'Create user'));
        setText('th_user', txt('th_user', 'User'));
        setText('th_role', txt('th_role', 'Role'));
        setText('th_email', txt('th_email', 'Email'));
        setText('th_password', txt('th_password', 'Password'));
        setText('th_banned', txt('th_banned', 'Banned'));
        setText('th_actions', txt('th_actions', 'Actions'));
        setText('admin_audit_title', txt('admin_audit_title', 'User behavior logs'));
        setText('admin_audit_sub', txt('admin_audit_sub', 'Recent audit events (server-side).'));
        setText('admin_refresh_audit_btn', txt('admin_refresh_audit_btn', 'Refresh logs'));

        const meResp = await getJson('/auth/me');
        if (!meResp.ok || !meResp.data || !meResp.data.logged_in || !meResp.data.is_admin) {
            location.href = '/';
            return;
        }
        bindDirtyTracking();
        bindLeaveGuard();

        document.getElementById('admin_logout_btn').addEventListener('click', async () => {
            await postJson('/auth/logout', {});
            location.href = '/login';
        });

        document.getElementById('admin_save_settings_btn').addEventListener('click', async () => {
            showHint('admin_settings_hint', txt('auth_working', 'Working...'), false);
            const body = {
                login_enabled: !!document.getElementById('admin_toggle_login').checked,
                registration_enabled: !!document.getElementById('admin_toggle_registration').checked,
                guest_enabled: !!document.getElementById('admin_toggle_guest').checked,
                non_admin_login_enabled: !!document.getElementById('admin_toggle_non_admin_login').checked,
                email_code_dev_mode: !!document.getElementById('admin_toggle_email_dev_mode').checked,
                email_code_ttl_secs: parseInt(readVal('admin_email_ttl_secs'), 10) || 600,
                email_code_resend_cooldown_secs: parseInt(readVal('admin_email_cooldown_secs'), 10) || 60,
                email_issue_per_min: parseInt(readVal('admin_email_issue_per_min'), 10) || 3,
                email_verify_per_min: parseInt(readVal('admin_email_verify_per_min'), 10) || 12,
                email_domain_allowlist_enabled: !!document.getElementById('admin_toggle_email_domain_allowlist_enabled').checked,
                email_domain_allowlist: String(document.getElementById('admin_email_domain_allowlist')?.value || '')
                    .split(/\r?\n/)
                    .map(x => String(x || '').trim())
                    .filter(Boolean),
                smtp_enabled: !!document.getElementById('admin_toggle_smtp_enabled').checked,
                smtp_starttls: !!document.getElementById('admin_toggle_smtp_starttls').checked,
                smtp_host: readVal('admin_smtp_host'),
                smtp_port: parseInt(readVal('admin_smtp_port'), 10) || 587,
                smtp_username: readVal('admin_smtp_username'),
                smtp_password: readVal('admin_smtp_password'),
                smtp_from: readVal('admin_smtp_from'),
            };
            const r = await postJson('/admin/api/settings', body);
            if (!r.ok) {
                showHint('admin_settings_hint', r.text || `HTTP ${r.status}`, true);
            } else {
                showHint('admin_settings_hint', txt('status_saved', 'Saved.'), false);
                setSettingsDirty(false);
            }
        });

        document.getElementById('admin_create_user_btn').addEventListener('click', async () => {
            showHint('admin_create_user_hint', txt('auth_working', 'Working...'), false);
            const r = await postJson('/admin/api/users', {
                username: readVal('admin_new_username'),
                nickname: readVal('admin_new_nickname'),
                email: readVal('admin_new_email'),
                role: readVal('admin_new_role'),
                password: readVal('admin_new_password'),
            });
            if (!r.ok) {
                showHint('admin_create_user_hint', r.text || `HTTP ${r.status}`, true);
                return;
            }
            showHint('admin_create_user_hint', txt('admin_user_created', 'User created.'), false);
            await refreshAll();
        });

        document.getElementById('admin_refresh_audit_btn').addEventListener('click', async () => {
            await refreshAll();
        });

        await refreshAll();
    }

    main().catch(e => {
        console.error(e);
        showHint('admin_settings_hint', String(e && e.message ? e.message : e), true);
    });
})();
