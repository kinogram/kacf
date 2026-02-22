//! Multi-user authentication and authorization (admin/user/guest).
//!
//! NOTE: This is a local-first app. Requirements explicitly ask that admins can
//! view other users' passwords, so passwords are stored in plaintext in the local
//! user database. Do not expose this server to untrusted networks.

mod email;
mod pages;
mod routes;
pub(crate) mod session;
mod store;
pub(crate) mod types;

pub(crate) use pages::{account_js, admin_js, auth_js};
pub(crate) use pages::{account_page, admin_page, login_page};
pub(crate) use routes::{
    account_change_login_option, account_change_password, account_change_username,
    account_confirm_email_change, account_request_current_email_code,
    account_request_new_email_code, account_self, account_update_profile, admin_audit_tail,
    admin_create_user, admin_delete_user, admin_get_settings, admin_list_users, admin_put_settings,
    admin_set_banned, admin_set_notice, admin_set_password, auth_me, bootstrap_admin,
    bootstrap_status, guest_start, login, logout, register, verify_email_code,
};
pub(crate) use store::{AuthStore, AuthSystemPaths};
pub(crate) use types::{AccountRole, SessionRecord, UserRecord};
