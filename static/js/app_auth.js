// Auth state + account menu UI.
// Loaded before runtime init; exposes KACF.auth.* helpers.

window.KACF = window.KACF || {};
window.KACF.auth = window.KACF.auth || {};

let authMe = null;
let authGuestMode = false;
const PROJECT_LINKS = {
    githubRepo: 'https://github.com',
    developerHome: 'https://example.com',
    donationInfo: 'https://example.com/donate',
    donationThanks: 'https://example.com/donate/thanks',
};

function isSafeUsernameForPath(u) {
    return /^[a-zA-Z0-9._-]{1,64}$/.test(String(u || ''));
}

async function fetchAuthMe() {
    const resp = await fetch('/auth/me', { cache: 'no-store' });
    if (!resp.ok) throw new Error(`auth/me failed: ${resp.status}`);
    return await resp.json();
}

function setGuestMode(on) {
    authGuestMode = !!on;
    try {
        window.KACF.auth.guest = authGuestMode;
    } catch (_e) {}
}

function isGuestMode() {
    return !!authGuestMode;
}

function setWorkspaceRootForUser(username) {
    if (!isSafeUsernameForPath(username)) {
        WORKSPACE_ROOT = './autocoding_data/workspaces';
        return;
    }
    WORKSPACE_ROOT = `./autocoding_data/users/${username}/workspaces`;
}

function closeAccountMenu() {
    const menu = document.getElementById('account_menu');
    if (menu) menu.style.display = 'none';
}

function toggleAccountMenu() {
    const menu = document.getElementById('account_menu');
    if (!menu) return;
    const shown = menu.style.display !== 'none';
    menu.style.display = shown ? 'none' : 'block';
}

function renderAccountMenu() {
    const me = authMe;
    const btn = document.getElementById('open_account_btn');
    if (btn) setIconButton(btn, 'user', txt('btn_account', 'Account'));
    const topAdminBtn = document.getElementById('open_admin_btn');

    const menuTitle = document.getElementById('account_menu_title');
    const menuBody = document.getElementById('account_menu_body');
    const manageBtn = document.getElementById('account_manage_btn');
    const switchBtn = document.getElementById('account_switch_btn');
    const logoutBtn = document.getElementById('account_logout_btn');
    const exitGuestBtn = document.getElementById('account_exit_guest_btn');
    const githubBtn = document.getElementById('account_github_btn');
    const devhomeBtn = document.getElementById('account_devhome_btn');
    const donateBtn = document.getElementById('account_donate_btn');
    const thanksBtn = document.getElementById('account_thanks_btn');

    const name = me && me.nickname ? me.nickname : txt('account_guest_name', 'Guest');
    if (menuTitle) menuTitle.textContent = fmt('account_menu_greeting', '{name}, hello!', { name });
    if (menuBody) menuBody.textContent = txt('account_menu_body', '');

    const isAdmin = !!(me && me.is_admin);
    const isGuest = !!(me && me.guest);
    if (topAdminBtn) {
        topAdminBtn.style.display = isAdmin ? '' : 'none';
        setIconButton(topAdminBtn, 'shield', txt('btn_admin_panel', 'Admin Panel'));
    }

    if (manageBtn) manageBtn.textContent = txt('account_menu_manage', 'Manage your KACF account');
    if (switchBtn) switchBtn.textContent = txt('account_menu_switch', 'Switch account');
    if (logoutBtn) logoutBtn.textContent = txt('account_menu_logout', 'Logout');
    if (githubBtn) githubBtn.textContent = txt('account_menu_github', 'GitHub: KACF');
    if (devhomeBtn) devhomeBtn.textContent = txt('account_menu_devhome', 'Developer home');
    if (donateBtn) donateBtn.textContent = txt('account_menu_donate', 'Donation details');
    if (thanksBtn) thanksBtn.textContent = txt('account_menu_thanks', 'Donation thanks list');
    if (exitGuestBtn) exitGuestBtn.textContent = txt('account_menu_exit_guest', 'Exit guest mode');

    const adminBtn = document.getElementById('account_admin_panel_btn');
    if (adminBtn) adminBtn.style.display = 'none';

    if (exitGuestBtn) exitGuestBtn.style.display = isGuest ? '' : 'none';
    if (manageBtn) manageBtn.style.display = isGuest ? 'none' : '';
    if (switchBtn) switchBtn.style.display = isGuest ? '' : '';
    if (logoutBtn) logoutBtn.style.display = isGuest ? 'none' : '';

    try {
        if (manageBtn) setButtonWithIcon(manageBtn, 'user', manageBtn.textContent);
        if (switchBtn) setButtonWithIcon(switchBtn, 'chevron-right', switchBtn.textContent);
        if (logoutBtn && !isGuest) setButtonWithIcon(logoutBtn, 'x', logoutBtn.textContent);
        if (exitGuestBtn && isGuest) setButtonWithIcon(exitGuestBtn, 'x', exitGuestBtn.textContent);
        if (githubBtn) setButtonWithIcon(githubBtn, 'link', githubBtn.textContent);
        if (devhomeBtn) setButtonWithIcon(devhomeBtn, 'link', devhomeBtn.textContent);
        if (donateBtn) setButtonWithIcon(donateBtn, 'link', donateBtn.textContent);
        if (thanksBtn) setButtonWithIcon(thanksBtn, 'link', thanksBtn.textContent);
    } catch (_e) {}

    if (manageBtn) manageBtn.onclick = () => window.open('/account', '_blank', 'noopener');
    if (topAdminBtn) topAdminBtn.onclick = () => window.open('/admin', '_blank', 'noopener');
    if (switchBtn) switchBtn.onclick = () => { location.href = '/login?switch=1'; };
    if (logoutBtn) logoutBtn.onclick = async () => {
        await fetch('/auth/logout', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' });
        location.href = '/login';
    };
    if (exitGuestBtn) exitGuestBtn.onclick = async () => {
        await fetch('/auth/logout', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' });
        location.href = '/login';
    };
    if (githubBtn) githubBtn.onclick = () => window.open(PROJECT_LINKS.githubRepo, '_blank', 'noopener');
    if (devhomeBtn) devhomeBtn.onclick = () => window.open(PROJECT_LINKS.developerHome, '_blank', 'noopener');
    if (donateBtn) donateBtn.onclick = () => window.open(PROJECT_LINKS.donationInfo, '_blank', 'noopener');
    if (thanksBtn) thanksBtn.onclick = () => window.open(PROJECT_LINKS.donationThanks, '_blank', 'noopener');

    if (btn && !btn.dataset.accountWired) {
        btn.dataset.accountWired = '1';
        btn.addEventListener('click', (e) => {
            e.preventDefault();
            e.stopPropagation();
            toggleAccountMenu();
        });
        document.addEventListener('click', () => closeAccountMenu());
        document.addEventListener('keydown', (ev) => {
            if (ev.key === 'Escape') closeAccountMenu();
        });
        const menu = document.getElementById('account_menu');
        if (menu) {
            menu.addEventListener('click', (e) => e.stopPropagation());
        }
    }
    if (topAdminBtn && !topAdminBtn.dataset.adminWired) {
        topAdminBtn.dataset.adminWired = '1';
        topAdminBtn.addEventListener('click', (e) => e.stopPropagation());
    }
}

async function ensureAuthForApp() {
    const me = await fetchAuthMe();
    authMe = me;
    window.KACF.auth.me = me;
    if (!me || !me.logged_in) {
        location.href = '/login';
        return null;
    }
    if (me.guest) {
        setGuestMode(true);
        try { window.KACF.state && window.KACF.state.setGuestModeLocked && window.KACF.state.setGuestModeLocked(true); } catch (_e) {}
    } else {
        setGuestMode(false);
        try { window.KACF.state && window.KACF.state.setGuestModeLocked && window.KACF.state.setGuestModeLocked(false); } catch (_e) {}
        setWorkspaceRootForUser(me.username || '');
    }
    return me;
}

window.KACF.auth.ensureAuthForApp = ensureAuthForApp;
window.KACF.auth.renderAccountMenu = renderAccountMenu;
window.KACF.auth.isGuestMode = isGuestMode;
