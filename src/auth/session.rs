use actix_web::cookie::{time::Duration, Cookie, SameSite};
use actix_web::HttpRequest;

pub(crate) const SESSION_COOKIE_NAME: &str = "kacf_session";

pub(crate) fn read_session_cookie(req: &HttpRequest) -> Option<String> {
    req.cookie(SESSION_COOKIE_NAME)
        .map(|c| c.value().to_string())
}

pub(crate) fn build_session_cookie(session_id: &str) -> Cookie<'static> {
    Cookie::build(SESSION_COOKIE_NAME, session_id.to_string())
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        // This server is often used via reverse tunnels; don't force Secure here.
        .max_age(Duration::days(7))
        .finish()
}

pub(crate) fn build_clear_session_cookie() -> Cookie<'static> {
    Cookie::build(SESSION_COOKIE_NAME, "")
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(Duration::seconds(0))
        .finish()
}
