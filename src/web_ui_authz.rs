use actix_web::{HttpRequest, HttpResponse};
use std::fs;

use crate::auth;
use crate::auth::{AccountRole, SessionRecord, UserRecord};
use crate::web_ui::AppState;

#[derive(Debug, Clone)]
pub(crate) struct WebUserCtx {
    pub(crate) role: AccountRole,
    pub(crate) username: Option<String>,
    pub(crate) user: Option<UserRecord>,
    pub(crate) managed_root_dir: String, // relative path for store helpers
    pub(crate) can_write: bool,
}

pub(crate) fn require_session(req: &HttpRequest, data: &AppState) -> Result<SessionRecord, HttpResponse> {
    let sid = auth::session::read_session_cookie(req).unwrap_or_default();
    if sid.is_empty() {
        return Err(HttpResponse::Unauthorized().body("not logged in"));
    }
    data.auth
        .resolve_session(&sid)
        .ok_or_else(|| HttpResponse::Unauthorized().body("invalid session"))
}

pub(crate) fn user_ctx_for_request(req: &HttpRequest, data: &AppState) -> Result<WebUserCtx, HttpResponse> {
    let sess = require_session(req, data)?;
    match sess.role {
        AccountRole::Guest => Ok(WebUserCtx {
            role: AccountRole::Guest,
            username: None,
            user: None,
            managed_root_dir: "autocoding_data/guest".to_string(),
            can_write: false,
        }),
        AccountRole::Admin | AccountRole::User => {
            let Some(username) = sess.username.clone() else {
                return Err(HttpResponse::Unauthorized().body("invalid session user"));
            };
            let Some(user) = data.auth.find_user_by_username(&username) else {
                return Err(HttpResponse::Unauthorized().body("unknown user"));
            };
            if user.banned {
                return Err(HttpResponse::Forbidden().body("account banned"));
            }
            let root = format!("autocoding_data/users/{}", username);
            Ok(WebUserCtx {
                role: user.role,
                username: Some(username),
                user: Some(user),
                managed_root_dir: root,
                can_write: true,
            })
        }
    }
}

pub(crate) fn ensure_user_dirs(ctx: &WebUserCtx) -> std::io::Result<()> {
    if !ctx.can_write {
        return Ok(());
    }
    let root = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(&ctx.managed_root_dir);
    fs::create_dir_all(root.join("workspaces"))?;
    Ok(())
}
