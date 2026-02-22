use std::collections::HashMap;
use std::path::PathBuf;

use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use serde::Serialize;

use crate::auth::session::{build_clear_session_cookie, build_session_cookie, read_session_cookie};
use crate::auth::store::AuthStore;
use crate::auth::types::{AccountRole, AuthMeResponse, LoginOption};

#[derive(Deserialize)]
pub(crate) struct BootstrapAdminPayload {
    pub(crate) username: String,
    pub(crate) nickname: String,
    pub(crate) email: String,
    pub(crate) password: String,
}

#[derive(Deserialize)]
pub(crate) struct RegisterPayload {
    pub(crate) username: String,
    pub(crate) nickname: String,
    pub(crate) email: String,
    pub(crate) password: String,
}

#[derive(Deserialize)]
pub(crate) struct LoginPayload {
    pub(crate) identifier: String, // username or email
    pub(crate) password: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct VerifyEmailCodePayload {
    pub(crate) identifier: String, // username or email
    pub(crate) code: String,
}

fn data_root() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("autocoding_data")
}

fn resolve_store(_req: &HttpRequest) -> AuthStore {
    // Store is created per request but internally uses file locks; lightweight.
    let paths = crate::auth::store::AuthSystemPaths::new(&data_root());
    let store = AuthStore::new(paths);
    let _ = store.ensure_dirs();
    store
}

fn email_allowed(email: &str) -> bool {
    // Simple allowlist to prevent mass account creation with random domains.
    let e = email.trim().to_lowercase();
    let Some((_user, domain)) = e.split_once('@') else {
        return false;
    };
    let allowed = [
        "gmail.com",
        "outlook.com",
        "hotmail.com",
        "live.com",
        "yahoo.com",
        "icloud.com",
        "proton.me",
        "qq.com",
        "163.com",
        "126.com",
        "foxmail.com",
    ];
    allowed.contains(&domain)
}

pub(crate) async fn bootstrap_status(req: HttpRequest) -> impl Responder {
    let store = resolve_store(&req);
    let any_user = store.has_any_user();
    HttpResponse::Ok().json(serde_json::json!({
        "has_any_user": any_user
    }))
}

pub(crate) async fn bootstrap_admin(
    req: HttpRequest,
    body: web::Json<BootstrapAdminPayload>,
) -> impl Responder {
    let store = resolve_store(&req);
    if store.has_any_user() {
        return HttpResponse::BadRequest().body("bootstrap already completed");
    }
    let payload = body.into_inner();
    if !email_allowed(&payload.email) {
        return HttpResponse::BadRequest().body("email not allowed");
    }
    if payload.password.trim().is_empty() {
        return HttpResponse::BadRequest().body("password is empty");
    }
    let u = match store.create_user(
        &payload.username,
        &payload.nickname,
        &payload.email,
        AccountRole::Admin,
        Some(payload.password),
        LoginOption::PasswordOnly,
    ) {
        Ok(v) => v,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };
    let mut fields = HashMap::new();
    fields.insert("username", u.username.clone());
    fields.insert("role", "admin".to_string());
    store.audit("bootstrap_admin", &fields);
    HttpResponse::Ok().body("ok")
}

pub(crate) async fn register(req: HttpRequest, body: web::Json<RegisterPayload>) -> impl Responder {
    let store = resolve_store(&req);
    let s = store.read_admin_settings();
    if !s.registration_enabled {
        return HttpResponse::Forbidden().body("registration disabled");
    }
    let payload = body.into_inner();
    if !email_allowed(&payload.email) {
        return HttpResponse::BadRequest().body("email not allowed");
    }
    if payload.password.trim().is_empty() {
        return HttpResponse::BadRequest().body("password is empty");
    }
    let u = match store.create_user(
        &payload.username,
        &payload.nickname,
        &payload.email,
        AccountRole::User,
        Some(payload.password),
        LoginOption::PasswordOnly,
    ) {
        Ok(v) => v,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };
    let mut fields = HashMap::new();
    fields.insert("username", u.username.clone());
    fields.insert("role", "user".to_string());
    store.audit("register", &fields);
    HttpResponse::Ok().body("ok")
}

pub(crate) async fn login(req: HttpRequest, body: web::Json<LoginPayload>) -> impl Responder {
    let store = resolve_store(&req);
    let s = store.read_admin_settings();
    if !s.login_enabled {
        return HttpResponse::Forbidden().body("login disabled");
    }
    let payload = body.into_inner();
    let ident = payload.identifier.trim();
    if ident.is_empty() {
        return HttpResponse::BadRequest().body("identifier is empty");
    }
    let user = if ident.contains('@') {
        store.find_user_by_email(ident)
    } else {
        store.find_user_by_username(ident)
    };
    let Some(user) = user else {
        return HttpResponse::Unauthorized().body("invalid credentials");
    };
    if user.role != AccountRole::Admin && !s.non_admin_login_enabled {
        return HttpResponse::Forbidden().body("non-admin login disabled");
    }
    if user.banned {
        return HttpResponse::Forbidden().body("account banned");
    }
    match user.login_option {
        LoginOption::PasswordOnly => {
            let pw = payload.password.unwrap_or_default();
            let ok = user.password.as_deref().unwrap_or("") == pw;
            if !ok {
                return HttpResponse::Unauthorized().body("invalid credentials");
            }
            let session = match store.create_session_for_user(&user) {
                Ok(v) => v,
                Err(e) => return HttpResponse::InternalServerError().body(e),
            };
            let mut fields = HashMap::new();
            fields.insert("username", user.username.clone());
            store.audit("login", &fields);
            HttpResponse::Ok()
                .cookie(build_session_cookie(&session.session_id))
                .json(serde_json::json!({"ok": true}))
        }
        LoginOption::EmailOnly => {
            let code = match store.create_email_code("login", &user.username, 10 * 60) {
                Ok(v) => v,
                Err(e) => return HttpResponse::InternalServerError().body(e),
            };
            let mut fields = HashMap::new();
            fields.insert("username", user.username.clone());
            store.audit("login_email_code_issued", &fields);
            HttpResponse::Conflict().json(serde_json::json!({
                "need": "email_code",
                "dev_code": code
            }))
        }
        LoginOption::PasswordEmail2fa => {
            let pw = payload.password.unwrap_or_default();
            let ok = user.password.as_deref().unwrap_or("") == pw;
            if !ok {
                return HttpResponse::Unauthorized().body("invalid credentials");
            }
            let code = match store.create_email_code("login", &user.username, 10 * 60) {
                Ok(v) => v,
                Err(e) => return HttpResponse::InternalServerError().body(e),
            };
            let mut fields = HashMap::new();
            fields.insert("username", user.username.clone());
            store.audit("login_password_ok_email_code_issued", &fields);
            HttpResponse::Conflict().json(serde_json::json!({
                "need": "email_code",
                "dev_code": code
            }))
        }
    }
}

pub(crate) async fn verify_email_code(
    req: HttpRequest,
    body: web::Json<VerifyEmailCodePayload>,
) -> impl Responder {
    let store = resolve_store(&req);
    let s = store.read_admin_settings();
    if !s.login_enabled {
        return HttpResponse::Forbidden().body("login disabled");
    }
    let payload = body.into_inner();
    let ident = payload.identifier.trim();
    if ident.is_empty() {
        return HttpResponse::BadRequest().body("identifier is empty");
    }
    let user = if ident.contains('@') {
        store.find_user_by_email(ident)
    } else {
        store.find_user_by_username(ident)
    };
    let Some(user) = user else {
        return HttpResponse::Unauthorized().body("invalid credentials");
    };
    if user.role != AccountRole::Admin && !s.non_admin_login_enabled {
        return HttpResponse::Forbidden().body("non-admin login disabled");
    }
    if user.banned {
        return HttpResponse::Forbidden().body("account banned");
    }
    let ok = store.consume_email_code("login", &user.username, &payload.code);
    if !ok {
        return HttpResponse::Unauthorized().body("invalid code");
    }
    let session = match store.create_session_for_user(&user) {
        Ok(v) => v,
        Err(e) => return HttpResponse::InternalServerError().body(e),
    };
    let mut fields = HashMap::new();
    fields.insert("username", user.username.clone());
    store.audit("login_email_code_verified", &fields);
    HttpResponse::Ok()
        .cookie(build_session_cookie(&session.session_id))
        .json(serde_json::json!({"ok": true}))
}

pub(crate) async fn guest_start(req: HttpRequest) -> impl Responder {
    let store = resolve_store(&req);
    let s = store.read_admin_settings();
    if !s.guest_enabled {
        return HttpResponse::Forbidden().body("guest disabled");
    }
    let session = match store.create_guest_session() {
        Ok(v) => v,
        Err(e) => return HttpResponse::InternalServerError().body(e),
    };
    store.audit("guest_start", &HashMap::new());
    HttpResponse::Ok()
        .cookie(build_session_cookie(&session.session_id))
        .json(serde_json::json!({"ok": true}))
}

pub(crate) async fn logout(req: HttpRequest) -> impl Responder {
    let store = resolve_store(&req);
    if let Some(sid) = read_session_cookie(&req) {
        let _ = store.delete_session(&sid);
    }
    HttpResponse::Ok()
        .cookie(build_clear_session_cookie())
        .json(serde_json::json!({"ok": true}))
}

pub(crate) async fn auth_me(req: HttpRequest) -> impl Responder {
    let store = resolve_store(&req);
    let sid = read_session_cookie(&req).unwrap_or_default();
    let session = store.resolve_session(&sid);
    let mut resp = AuthMeResponse {
        logged_in: false,
        role: AccountRole::Guest,
        username: None,
        nickname: "Guest".to_string(),
        email: None,
        is_admin: false,
        guest: true,
        banned: false,
        forced_notice: None,
        forced_notice_min_seconds: 0,
    };
    if let Some(session) = session {
        resp.logged_in = true;
        resp.role = session.role;
        resp.guest = session.role == AccountRole::Guest;
        resp.username = session.username.clone();
        if session.role == AccountRole::Guest {
            resp.nickname = "Guest".to_string();
            resp.is_admin = false;
        } else if let Some(mut u) = session
            .username
            .as_deref()
            .and_then(|x| store.find_user_by_username(x))
        {
            resp.nickname = u.nickname.clone();
            resp.email = Some(u.email.clone());
            resp.is_admin = u.role == AccountRole::Admin;
            resp.banned = u.banned;
            resp.forced_notice = u.forced_notice.clone();
            resp.forced_notice_min_seconds = u.forced_notice_min_seconds;
            // One-shot: once delivered to the client, clear it.
            if u.forced_notice.is_some() {
                u.forced_notice = None;
                u.forced_notice_min_seconds = 0;
                let _ = store.upsert_user(u);
            }
        }
    }
    HttpResponse::Ok().json(resp)
}

fn require_admin(
    req: &HttpRequest,
    store: &AuthStore,
) -> Result<crate::auth::UserRecord, HttpResponse> {
    let sid = read_session_cookie(req).unwrap_or_default();
    let Some(sess) = store.resolve_session(&sid) else {
        return Err(HttpResponse::Unauthorized().body("not logged in"));
    };
    let Some(username) = sess.username.as_deref() else {
        return Err(HttpResponse::Unauthorized().body("invalid session"));
    };
    let Some(user) = store.find_user_by_username(username) else {
        return Err(HttpResponse::Unauthorized().body("unknown user"));
    };
    if user.role != AccountRole::Admin {
        return Err(HttpResponse::Forbidden().body("admin only"));
    }
    Ok(user)
}

pub(crate) async fn admin_get_settings(req: HttpRequest) -> impl Responder {
    let store = resolve_store(&req);
    if require_admin(&req, &store).is_err() {
        return HttpResponse::Forbidden().body("admin only");
    }
    HttpResponse::Ok().json(store.read_admin_settings())
}

pub(crate) async fn admin_put_settings(
    req: HttpRequest,
    body: web::Json<crate::auth::types::AdminSettings>,
) -> impl Responder {
    let store = resolve_store(&req);
    let admin = match require_admin(&req, &store) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let next = body.into_inner();
    if let Err(e) = store.write_admin_settings(&next) {
        return HttpResponse::InternalServerError().body(format!("save settings failed: {}", e));
    }
    let mut fields = HashMap::new();
    fields.insert("by", admin.username);
    store.audit("admin_put_settings", &fields);
    HttpResponse::Ok().body("ok")
}

pub(crate) async fn admin_list_users(req: HttpRequest) -> impl Responder {
    let store = resolve_store(&req);
    if require_admin(&req, &store).is_err() {
        return HttpResponse::Forbidden().body("admin only");
    }
    HttpResponse::Ok().json(store.list_users())
}

#[derive(Deserialize)]
pub(crate) struct AdminCreateUserPayload {
    pub(crate) username: String,
    pub(crate) nickname: String,
    pub(crate) email: String,
    pub(crate) role: String,
    pub(crate) password: String,
}

pub(crate) async fn admin_create_user(
    req: HttpRequest,
    body: web::Json<AdminCreateUserPayload>,
) -> impl Responder {
    let store = resolve_store(&req);
    let admin = match require_admin(&req, &store) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let p = body.into_inner();
    let role = if p.role.trim().eq_ignore_ascii_case("admin") {
        AccountRole::Admin
    } else {
        AccountRole::User
    };
    if !email_allowed(&p.email) {
        return HttpResponse::BadRequest().body("email not allowed");
    }
    let pw = if p.password.trim().is_empty() {
        None
    } else {
        Some(p.password)
    };
    let u = match store.create_user(
        &p.username,
        &p.nickname,
        &p.email,
        role,
        pw,
        LoginOption::PasswordOnly,
    ) {
        Ok(v) => v,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };
    let mut fields = HashMap::new();
    fields.insert("by", admin.username);
    fields.insert("username", u.username.clone());
    fields.insert("role", format!("{:?}", u.role));
    store.audit("admin_create_user", &fields);
    HttpResponse::Ok().json(u)
}

pub(crate) async fn admin_delete_user(req: HttpRequest, path: web::Path<String>) -> impl Responder {
    let store = resolve_store(&req);
    let admin = match require_admin(&req, &store) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let username = path.into_inner();
    if username.trim().is_empty() {
        return HttpResponse::BadRequest().body("username is empty");
    }
    if username == admin.username {
        return HttpResponse::BadRequest().body("cannot delete yourself");
    }
    match store.delete_user(&username) {
        Ok(_) => {
            let mut fields = HashMap::new();
            fields.insert("by", admin.username);
            fields.insert("username", username);
            store.audit("admin_delete_user", &fields);
            HttpResponse::Ok().body("deleted")
        }
        Err(e) => HttpResponse::InternalServerError().body(format!("delete failed: {}", e)),
    }
}

#[derive(Deserialize)]
pub(crate) struct AdminBanPayload {
    pub(crate) banned: bool,
}

pub(crate) async fn admin_set_banned(
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<AdminBanPayload>,
) -> impl Responder {
    let store = resolve_store(&req);
    let admin = match require_admin(&req, &store) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let username = path.into_inner();
    if username == admin.username {
        return HttpResponse::BadRequest().body("cannot ban yourself");
    }
    let banned = body.into_inner().banned;
    match store.set_user_banned(&username, banned) {
        Ok(_) => {
            let mut fields = HashMap::new();
            fields.insert("by", admin.username);
            fields.insert("username", username);
            fields.insert("banned", banned.to_string());
            store.audit("admin_set_banned", &fields);
            HttpResponse::Ok().body("ok")
        }
        Err(e) => HttpResponse::InternalServerError().body(format!("update failed: {}", e)),
    }
}

#[derive(Deserialize)]
pub(crate) struct AdminNoticePayload {
    pub(crate) message: String,
    pub(crate) min_seconds: u32,
}

pub(crate) async fn admin_set_notice(
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<AdminNoticePayload>,
) -> impl Responder {
    let store = resolve_store(&req);
    let admin = match require_admin(&req, &store) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let username = path.into_inner();
    if username.trim().is_empty() {
        return HttpResponse::BadRequest().body("username is empty");
    }
    if username == admin.username {
        return HttpResponse::BadRequest().body("cannot set notice for yourself");
    }
    let mut user = match store.find_user_by_username(&username) {
        Some(u) => u,
        None => return HttpResponse::NotFound().body("user not found"),
    };
    let msg = body.message.trim().to_string();
    if msg.is_empty() {
        user.forced_notice = None;
        user.forced_notice_min_seconds = 0;
    } else {
        user.forced_notice = Some(msg);
        user.forced_notice_min_seconds = body.min_seconds.min(300);
    }
    if let Err(e) = store.upsert_user(user) {
        return HttpResponse::InternalServerError().body(format!("save failed: {}", e));
    }
    let mut fields = HashMap::new();
    fields.insert("by", admin.username);
    fields.insert("username", username);
    store.audit("admin_set_notice", &fields);
    HttpResponse::Ok().body("ok")
}

#[derive(Deserialize)]
pub(crate) struct AdminSetPasswordPayload {
    pub(crate) password: String,
}

pub(crate) async fn admin_set_password(
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<AdminSetPasswordPayload>,
) -> impl Responder {
    let store = resolve_store(&req);
    let admin = match require_admin(&req, &store) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let username = path.into_inner();
    if username.trim().is_empty() {
        return HttpResponse::BadRequest().body("username is empty");
    }
    if username == admin.username {
        return HttpResponse::BadRequest().body("use account page to change your own password");
    }
    let mut user = match store.find_user_by_username(&username) {
        Some(u) => u,
        None => return HttpResponse::NotFound().body("user not found"),
    };
    let pw = body.password.trim().to_string();
    if pw.is_empty() {
        user.password = None;
    } else {
        user.password = Some(pw);
    }
    if let Err(e) = store.upsert_user(user) {
        return HttpResponse::InternalServerError().body(format!("save failed: {}", e));
    }
    let mut fields = HashMap::new();
    fields.insert("by", admin.username);
    fields.insert("username", username);
    store.audit("admin_set_password", &fields);
    HttpResponse::Ok().body("ok")
}

pub(crate) async fn admin_audit_tail(
    req: HttpRequest,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> impl Responder {
    let store = resolve_store(&req);
    if require_admin(&req, &store).is_err() {
        return HttpResponse::Forbidden().body("admin only");
    }
    let n: usize = query
        .get("tail")
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    let n = n.clamp(1, 2000);
    let content = std::fs::read_to_string(&store.paths().audit_log).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(n);
    let mut out = Vec::new();
    for line in &lines[start..] {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            out.push(v);
        }
    }
    HttpResponse::Ok().json(out)
}

#[derive(Deserialize)]
pub(crate) struct AccountProfilePayload {
    pub(crate) nickname: String,
}

pub(crate) async fn account_update_profile(
    req: HttpRequest,
    body: web::Json<AccountProfilePayload>,
) -> impl Responder {
    let store = resolve_store(&req);
    let sid = read_session_cookie(&req).unwrap_or_default();
    let Some(sess) = store.resolve_session(&sid) else {
        return HttpResponse::Unauthorized().body("not logged in");
    };
    if sess.role == AccountRole::Guest {
        return HttpResponse::Forbidden().body("guest session");
    }
    let Some(username) = sess.username.as_deref() else {
        return HttpResponse::Unauthorized().body("invalid session");
    };
    let mut user = match store.find_user_by_username(username) {
        Some(u) => u,
        None => return HttpResponse::Unauthorized().body("unknown user"),
    };
    if user.banned {
        return HttpResponse::Forbidden().body("account banned");
    }
    let nickname = body.nickname.trim().to_string();
    if nickname.is_empty() {
        return HttpResponse::BadRequest().body("nickname is empty");
    }
    // Non-admin accounts cannot use admin-like nicknames.
    if user.role != AccountRole::Admin && crate::auth::store::nickname_reserved(&nickname) {
        return HttpResponse::BadRequest().body("nickname not allowed");
    }
    user.nickname = nickname;
    if let Err(e) = store.upsert_user(user) {
        return HttpResponse::InternalServerError().body(format!("save failed: {}", e));
    }
    HttpResponse::Ok().body("ok")
}

#[derive(Deserialize)]
pub(crate) struct AccountChangePasswordPayload {
    pub(crate) old_password: String,
    pub(crate) new_password: String,
    pub(crate) current_email_code: Option<String>,
}

pub(crate) async fn account_change_password(
    req: HttpRequest,
    body: web::Json<AccountChangePasswordPayload>,
) -> impl Responder {
    let store = resolve_store(&req);
    let sid = read_session_cookie(&req).unwrap_or_default();
    let Some(sess) = store.resolve_session(&sid) else {
        return HttpResponse::Unauthorized().body("not logged in");
    };
    if sess.role == AccountRole::Guest {
        return HttpResponse::Forbidden().body("guest session");
    }
    let Some(username) = sess.username.as_deref() else {
        return HttpResponse::Unauthorized().body("invalid session");
    };
    let mut user = match store.find_user_by_username(username) {
        Some(u) => u,
        None => return HttpResponse::Unauthorized().body("unknown user"),
    };
    if user.banned {
        return HttpResponse::Forbidden().body("account banned");
    }
    let p = body.into_inner();
    let old_pw = p.old_password;
    let new_pw = p.new_password;
    if !user.login_option.requires_password() {
        return HttpResponse::BadRequest().body("password not enabled for this account");
    }
    if let Err(e) = verify_identity(
        &user,
        &store,
        Some(&old_pw),
        p.current_email_code.as_deref(),
    ) {
        return HttpResponse::Unauthorized().body(e);
    }
    if new_pw.trim().is_empty() {
        return HttpResponse::BadRequest().body("new password is empty");
    }
    user.password = Some(new_pw);
    if let Err(e) = store.upsert_user(user) {
        return HttpResponse::InternalServerError().body(format!("save failed: {}", e));
    }
    HttpResponse::Ok().body("ok")
}

#[derive(Serialize)]
pub(crate) struct AccountSelfResponse {
    username: String,
    nickname: String,
    email: String,
    role: AccountRole,
    banned: bool,
    login_option: LoginOption,
}

pub(crate) async fn account_self(req: HttpRequest) -> impl Responder {
    let store = resolve_store(&req);
    let sid = read_session_cookie(&req).unwrap_or_default();
    let Some(sess) = store.resolve_session(&sid) else {
        return HttpResponse::Unauthorized().body("not logged in");
    };
    if sess.role == AccountRole::Guest {
        return HttpResponse::Forbidden().body("guest session");
    }
    let Some(username) = sess.username.as_deref() else {
        return HttpResponse::Unauthorized().body("invalid session");
    };
    let user = match store.find_user_by_username(username) {
        Some(u) => u,
        None => return HttpResponse::Unauthorized().body("unknown user"),
    };
    HttpResponse::Ok().json(AccountSelfResponse {
        username: user.username,
        nickname: user.nickname,
        email: user.email,
        role: user.role,
        banned: user.banned,
        login_option: user.login_option,
    })
}

fn username_allowed(username: &str) -> bool {
    let u = username.trim();
    if u.is_empty() || u.len() > 64 {
        return false;
    }
    u.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

fn verify_identity(
    user: &crate::auth::UserRecord,
    store: &AuthStore,
    old_password: Option<&str>,
    current_email_code: Option<&str>,
) -> Result<(), String> {
    if user.login_option.requires_password() {
        let pw = old_password.unwrap_or("").trim();
        if user.password.as_deref().unwrap_or("") != pw {
            return Err("old password incorrect".to_string());
        }
    }
    if user.login_option.requires_email_code() {
        let code = current_email_code.unwrap_or("").trim();
        if !store.consume_email_code("verify_current_email", &user.username, code) {
            return Err("invalid current email code".to_string());
        }
    }
    Ok(())
}

pub(crate) async fn account_request_current_email_code(req: HttpRequest) -> impl Responder {
    let store = resolve_store(&req);
    let sid = read_session_cookie(&req).unwrap_or_default();
    let Some(sess) = store.resolve_session(&sid) else {
        return HttpResponse::Unauthorized().body("not logged in");
    };
    if sess.role == AccountRole::Guest {
        return HttpResponse::Forbidden().body("guest session");
    }
    let Some(username) = sess.username.as_deref() else {
        return HttpResponse::Unauthorized().body("invalid session");
    };
    let user = match store.find_user_by_username(username) {
        Some(u) => u,
        None => return HttpResponse::Unauthorized().body("unknown user"),
    };
    if user.banned {
        return HttpResponse::Forbidden().body("account banned");
    }
    let code = match store.create_email_code("verify_current_email", &user.username, 10 * 60) {
        Ok(v) => v,
        Err(e) => return HttpResponse::InternalServerError().body(e),
    };
    let mut fields = HashMap::new();
    fields.insert("username", user.username);
    store.audit("account_current_email_code_issued", &fields);
    HttpResponse::Ok().json(serde_json::json!({"dev_code": code}))
}

#[derive(Deserialize)]
pub(crate) struct AccountUsernamePayload {
    pub(crate) new_username: String,
    pub(crate) old_password: Option<String>,
    pub(crate) current_email_code: Option<String>,
}

pub(crate) async fn account_change_username(
    req: HttpRequest,
    body: web::Json<AccountUsernamePayload>,
) -> impl Responder {
    let store = resolve_store(&req);
    let sid = read_session_cookie(&req).unwrap_or_default();
    let Some(sess) = store.resolve_session(&sid) else {
        return HttpResponse::Unauthorized().body("not logged in");
    };
    if sess.role == AccountRole::Guest {
        return HttpResponse::Forbidden().body("guest session");
    }
    let Some(username) = sess.username.as_deref() else {
        return HttpResponse::Unauthorized().body("invalid session");
    };
    let user = match store.find_user_by_username(username) {
        Some(u) => u,
        None => return HttpResponse::Unauthorized().body("unknown user"),
    };
    if user.banned {
        return HttpResponse::Forbidden().body("account banned");
    }
    let p = body.into_inner();
    let new_u = p.new_username.trim();
    if !username_allowed(new_u) {
        return HttpResponse::BadRequest().body("username not allowed");
    }
    if let Err(e) = verify_identity(
        &user,
        &store,
        p.old_password.as_deref(),
        p.current_email_code.as_deref(),
    ) {
        return HttpResponse::Unauthorized().body(e);
    }
    let updated = match store.rename_user(&user.username, new_u) {
        Ok(v) => v,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };
    // Replace session cookie with a new session for renamed username.
    let session = match store.create_session_for_user(&updated) {
        Ok(v) => v,
        Err(e) => return HttpResponse::InternalServerError().body(e),
    };
    let mut fields = HashMap::new();
    fields.insert("old", user.username);
    fields.insert("new", updated.username.clone());
    store.audit("account_change_username", &fields);
    HttpResponse::Ok()
        .cookie(build_session_cookie(&session.session_id))
        .json(serde_json::json!({"ok": true, "username": updated.username}))
}

#[derive(Deserialize)]
pub(crate) struct AccountRequestNewEmailCodePayload {
    pub(crate) new_email: String,
}

pub(crate) async fn account_request_new_email_code(
    req: HttpRequest,
    body: web::Json<AccountRequestNewEmailCodePayload>,
) -> impl Responder {
    let store = resolve_store(&req);
    let sid = read_session_cookie(&req).unwrap_or_default();
    let Some(sess) = store.resolve_session(&sid) else {
        return HttpResponse::Unauthorized().body("not logged in");
    };
    if sess.role == AccountRole::Guest {
        return HttpResponse::Forbidden().body("guest session");
    }
    let Some(username) = sess.username.as_deref() else {
        return HttpResponse::Unauthorized().body("invalid session");
    };
    let user = match store.find_user_by_username(username) {
        Some(u) => u,
        None => return HttpResponse::Unauthorized().body("unknown user"),
    };
    if user.banned {
        return HttpResponse::Forbidden().body("account banned");
    }
    let new_email = body.new_email.trim().to_lowercase();
    if !email_allowed(&new_email) {
        return HttpResponse::BadRequest().body("email not allowed");
    }
    let key = format!("{}:{}", user.username, new_email);
    let code = match store.create_email_code("verify_new_email", &key, 10 * 60) {
        Ok(v) => v,
        Err(e) => return HttpResponse::InternalServerError().body(e),
    };
    let mut fields = HashMap::new();
    fields.insert("username", user.username);
    store.audit("account_new_email_code_issued", &fields);
    HttpResponse::Ok().json(serde_json::json!({"dev_code": code}))
}

#[derive(Deserialize)]
pub(crate) struct AccountConfirmEmailChangePayload {
    pub(crate) new_email: String,
    pub(crate) new_email_code: String,
    pub(crate) old_password: Option<String>,
    pub(crate) current_email_code: Option<String>,
}

pub(crate) async fn account_confirm_email_change(
    req: HttpRequest,
    body: web::Json<AccountConfirmEmailChangePayload>,
) -> impl Responder {
    let store = resolve_store(&req);
    let sid = read_session_cookie(&req).unwrap_or_default();
    let Some(sess) = store.resolve_session(&sid) else {
        return HttpResponse::Unauthorized().body("not logged in");
    };
    if sess.role == AccountRole::Guest {
        return HttpResponse::Forbidden().body("guest session");
    }
    let Some(username) = sess.username.as_deref() else {
        return HttpResponse::Unauthorized().body("invalid session");
    };
    let mut user = match store.find_user_by_username(username) {
        Some(u) => u,
        None => return HttpResponse::Unauthorized().body("unknown user"),
    };
    if user.banned {
        return HttpResponse::Forbidden().body("account banned");
    }
    let p = body.into_inner();
    if let Err(e) = verify_identity(
        &user,
        &store,
        p.old_password.as_deref(),
        p.current_email_code.as_deref(),
    ) {
        return HttpResponse::Unauthorized().body(e);
    }
    let new_email = p.new_email.trim().to_lowercase();
    if !email_allowed(&new_email) {
        return HttpResponse::BadRequest().body("email not allowed");
    }
    let key = format!("{}:{}", user.username, new_email);
    if !store.consume_email_code("verify_new_email", &key, &p.new_email_code) {
        return HttpResponse::Unauthorized().body("invalid new email code");
    }
    // Prevent duplicate email.
    if let Some(existing) = store.find_user_by_email(&new_email) {
        if existing.username != user.username {
            return HttpResponse::BadRequest().body("email already exists");
        }
    }
    user.email = new_email;
    if let Err(e) = store.upsert_user(user.clone()) {
        return HttpResponse::InternalServerError().body(format!("save failed: {}", e));
    }
    let mut fields = HashMap::new();
    fields.insert("username", user.username);
    store.audit("account_change_email", &fields);
    HttpResponse::Ok().body("ok")
}

#[derive(Deserialize)]
pub(crate) struct AccountChangeLoginOptionPayload {
    pub(crate) new_option: String,
    pub(crate) old_password: Option<String>,
    pub(crate) current_email_code: Option<String>,
    pub(crate) new_password: Option<String>,
}

pub(crate) async fn account_change_login_option(
    req: HttpRequest,
    body: web::Json<AccountChangeLoginOptionPayload>,
) -> impl Responder {
    let store = resolve_store(&req);
    let sid = read_session_cookie(&req).unwrap_or_default();
    let Some(sess) = store.resolve_session(&sid) else {
        return HttpResponse::Unauthorized().body("not logged in");
    };
    if sess.role == AccountRole::Guest {
        return HttpResponse::Forbidden().body("guest session");
    }
    let Some(username) = sess.username.as_deref() else {
        return HttpResponse::Unauthorized().body("invalid session");
    };
    let mut user = match store.find_user_by_username(username) {
        Some(u) => u,
        None => return HttpResponse::Unauthorized().body("unknown user"),
    };
    if user.banned {
        return HttpResponse::Forbidden().body("account banned");
    }
    let p = body.into_inner();
    if let Err(e) = verify_identity(
        &user,
        &store,
        p.old_password.as_deref(),
        p.current_email_code.as_deref(),
    ) {
        return HttpResponse::Unauthorized().body(e);
    }
    let next = match p.new_option.as_str() {
        "password_only" => LoginOption::PasswordOnly,
        "password_email_2fa" => LoginOption::PasswordEmail2fa,
        "email_only" => LoginOption::EmailOnly,
        _ => return HttpResponse::BadRequest().body("invalid login option"),
    };
    if next.requires_password() {
        if user.password.as_deref().unwrap_or("").is_empty() {
            let np = p.new_password.unwrap_or_default();
            if np.trim().is_empty() {
                return HttpResponse::BadRequest().body("password required for this login option");
            }
            user.password = Some(np);
        }
    } else {
        // No-password account: remove stored password.
        user.password = None;
    }
    user.login_option = next;
    if let Err(e) = store.upsert_user(user.clone()) {
        return HttpResponse::InternalServerError().body(format!("save failed: {}", e));
    }
    let mut fields = HashMap::new();
    fields.insert("username", user.username);
    fields.insert("login_option", format!("{:?}", user.login_option));
    store.audit("account_change_login_option", &fields);
    HttpResponse::Ok().body("ok")
}
