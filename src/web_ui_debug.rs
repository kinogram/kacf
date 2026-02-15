use actix_web::{web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::lock_utils::lock_recover;

#[derive(Debug, Clone, Serialize)]
pub struct ClientLogEntry {
    pub ts_unix: u64,
    pub level: String,
    pub message: String,
    pub href: String,
    pub user_agent: String,
    pub stack: String,
}

#[derive(Debug, Deserialize)]
pub struct ClientLogInput {
    pub level: Option<String>,
    pub message: Option<String>,
    pub href: Option<String>,
    pub user_agent: Option<String>,
    pub stack: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClientLogQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ClientLogListResponse {
    pub entries: Vec<ClientLogEntry>,
}

#[derive(Debug, Clone)]
pub struct DebugLogStore {
    entries: Arc<Mutex<Vec<ClientLogEntry>>>,
    bytes: Arc<Mutex<usize>>,
}

impl DebugLogStore {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(Vec::new())),
            bytes: Arc::new(Mutex::new(0)),
        }
    }

    pub fn entries(&self) -> Arc<Mutex<Vec<ClientLogEntry>>> {
        self.entries.clone()
    }

    pub fn bytes(&self) -> Arc<Mutex<usize>> {
        self.bytes.clone()
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn clamp_len(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    // Keep it ASCII-safe and deterministic.
    let mut out = s[..max].to_string();
    out.push_str("...");
    out
}

fn estimate_entry_bytes(e: &ClientLogEntry) -> usize {
    e.level.len() + e.message.len() + e.href.len() + e.user_agent.len() + e.stack.len() + 32
}

fn push_entry(
    entries: &mut Vec<ClientLogEntry>,
    bytes: &mut usize,
    entry: ClientLogEntry,
    max_entries: usize,
    max_bytes: usize,
) {
    let add = estimate_entry_bytes(&entry);
    entries.push(entry);
    *bytes = bytes.saturating_add(add);

    while entries.len() > max_entries || *bytes > max_bytes {
        if let Some(old) = entries.first() {
            *bytes = bytes.saturating_sub(estimate_entry_bytes(old));
        }
        if !entries.is_empty() {
            entries.remove(0);
        } else {
            break;
        }
    }
}

pub async fn post_client_log(
    store_entries: web::Data<Arc<Mutex<Vec<ClientLogEntry>>>>,
    store_bytes: web::Data<Arc<Mutex<usize>>>,
    body: web::Json<ClientLogInput>,
) -> impl Responder {
    let level = clamp_len(body.level.as_deref().unwrap_or("error"), 16);
    let message = clamp_len(body.message.as_deref().unwrap_or(""), 2000);
    let href = clamp_len(body.href.as_deref().unwrap_or(""), 512);
    let user_agent = clamp_len(body.user_agent.as_deref().unwrap_or(""), 512);
    let stack = clamp_len(body.stack.as_deref().unwrap_or(""), 8000);

    if message.is_empty() && stack.is_empty() {
        return HttpResponse::BadRequest().body("empty log");
    }

    let entry = ClientLogEntry {
        ts_unix: now_unix(),
        level,
        message,
        href,
        user_agent,
        stack,
    };

    let mut entries = lock_recover(
        store_entries.get_ref().as_ref(),
        "debug_client_logs.entries",
    );
    let mut bytes = lock_recover(store_bytes.get_ref().as_ref(), "debug_client_logs.bytes");
    push_entry(&mut entries, &mut bytes, entry, 200, 256 * 1024);
    HttpResponse::Ok().json(serde_json::json!({ "ok": true }))
}

pub async fn get_client_logs(
    store_entries: web::Data<Arc<Mutex<Vec<ClientLogEntry>>>>,
    query: web::Query<ClientLogQuery>,
) -> impl Responder {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let entries = lock_recover(
        store_entries.get_ref().as_ref(),
        "debug_client_logs.entries",
    );
    let slice = if entries.len() > limit {
        entries[entries.len() - limit..].to_vec()
    } else {
        entries.clone()
    };
    HttpResponse::Ok().json(ClientLogListResponse { entries: slice })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_len_truncates() {
        let s = "a".repeat(10);
        assert_eq!(clamp_len(&s, 20), s);
        let t = "b".repeat(30);
        assert_eq!(clamp_len(&t, 10), format!("{}...", "b".repeat(10)));
    }

    #[test]
    fn push_entry_evictions_work() {
        let mut entries = Vec::new();
        let mut bytes = 0usize;
        for i in 0..10 {
            push_entry(
                &mut entries,
                &mut bytes,
                ClientLogEntry {
                    ts_unix: i,
                    level: "e".into(),
                    message: "m".repeat(50),
                    href: "".into(),
                    user_agent: "".into(),
                    stack: "".into(),
                },
                5,
                10_000,
            );
        }
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0].ts_unix, 5);
        assert!(bytes > 0);
    }
}
