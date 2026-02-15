use actix_web::{HttpResponse, Responder};

const LOGIN_HTML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/static/login.html"));
const ACCOUNT_HTML: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/static/account.html"));
const ADMIN_HTML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/static/admin.html"));

const AUTH_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/static/js/auth.js"));
const ACCOUNT_JS: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/static/js/account.js"));
const ADMIN_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/static/js/admin.js"));

pub(crate) async fn login_page() -> impl Responder {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(LOGIN_HTML)
}

pub(crate) async fn account_page() -> impl Responder {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(ACCOUNT_HTML)
}

pub(crate) async fn admin_page() -> impl Responder {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(ADMIN_HTML)
}

pub(crate) async fn auth_js() -> impl Responder {
    HttpResponse::Ok()
        .content_type("application/javascript; charset=utf-8")
        .body(AUTH_JS)
}

pub(crate) async fn account_js() -> impl Responder {
    HttpResponse::Ok()
        .content_type("application/javascript; charset=utf-8")
        .body(ACCOUNT_JS)
}

pub(crate) async fn admin_js() -> impl Responder {
    HttpResponse::Ok()
        .content_type("application/javascript; charset=utf-8")
        .body(ADMIN_JS)
}

