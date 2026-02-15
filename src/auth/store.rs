use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};

use crate::auth::types::{AccountRole, AdminSettings, LoginOption, SessionRecord, UserRecord};

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn rand_id(prefix: &str, len: usize) -> String {
    let tail: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect();
    format!("{prefix}_{tail}")
}

#[derive(Clone)]
pub(crate) struct AuthSystemPaths {
    pub(crate) system_dir: PathBuf,
    pub(crate) users_db: PathBuf,
    pub(crate) sessions_db: PathBuf,
    pub(crate) admin_settings: PathBuf,
    pub(crate) audit_log: PathBuf,
    pub(crate) per_user_root_dir: PathBuf,
}

impl AuthSystemPaths {
    pub(crate) fn new(data_root: &Path) -> Self {
        let system_dir = data_root.join("system");
        let per_user_root_dir = data_root.join("users");
        Self {
            system_dir: system_dir.clone(),
            users_db: system_dir.join("users.json"),
            sessions_db: system_dir.join("sessions.json"),
            admin_settings: system_dir.join("admin_settings.json"),
            audit_log: system_dir.join("audit.log"),
            per_user_root_dir,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UsersDb {
    users: Vec<UserRecord>,
}

impl Default for UsersDb {
    fn default() -> Self {
        Self { users: vec![] }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionsDb {
    sessions: Vec<SessionRecord>,
}

impl Default for SessionsDb {
    fn default() -> Self {
        Self { sessions: vec![] }
    }
}

#[derive(Clone)]
pub(crate) struct AuthStore {
    paths: AuthSystemPaths,
    // Mutex for file-backed stores; this is a single-process local server.
    lock: Arc<Mutex<()>>,
}

impl AuthStore {
    pub(crate) fn new(paths: AuthSystemPaths) -> Self {
        Self {
            paths,
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) fn paths(&self) -> &AuthSystemPaths {
        &self.paths
    }

    pub(crate) fn ensure_dirs(&self) -> std::io::Result<()> {
        let _g = self.lock.lock().unwrap();
        fs::create_dir_all(&self.paths.system_dir)?;
        fs::create_dir_all(&self.paths.per_user_root_dir)?;
        if !self.paths.admin_settings.exists() {
            let json = serde_json::to_string_pretty(&AdminSettings::default())
                .unwrap_or_else(|_| "{}".to_string());
            atomic_write(&self.paths.admin_settings, json)?;
        }
        Ok(())
    }

    pub(crate) fn read_admin_settings(&self) -> AdminSettings {
        let _g = self.lock.lock().unwrap();
        read_json_or_default(&self.paths.admin_settings).unwrap_or_default()
    }

    pub(crate) fn write_admin_settings(&self, s: &AdminSettings) -> std::io::Result<()> {
        let _g = self.lock.lock().unwrap();
        let json = serde_json::to_string_pretty(s)?;
        atomic_write(&self.paths.admin_settings, json)
    }

    pub(crate) fn has_any_user(&self) -> bool {
        let _g = self.lock.lock().unwrap();
        let db: UsersDb = read_json_or_default(&self.paths.users_db).unwrap_or_default();
        !db.users.is_empty()
    }

    pub(crate) fn list_users(&self) -> Vec<UserRecord> {
        let _g = self.lock.lock().unwrap();
        let db: UsersDb = read_json_or_default(&self.paths.users_db).unwrap_or_default();
        db.users
    }

    pub(crate) fn find_user_by_username(&self, username: &str) -> Option<UserRecord> {
        let u = username.trim();
        if u.is_empty() {
            return None;
        }
        let _g = self.lock.lock().unwrap();
        let db: UsersDb = read_json_or_default(&self.paths.users_db).unwrap_or_default();
        db.users.into_iter().find(|x| x.username == u)
    }

    pub(crate) fn find_user_by_email(&self, email: &str) -> Option<UserRecord> {
        let e = email.trim().to_lowercase();
        if e.is_empty() {
            return None;
        }
        let _g = self.lock.lock().unwrap();
        let db: UsersDb = read_json_or_default(&self.paths.users_db).unwrap_or_default();
        db.users.into_iter().find(|x| x.email.to_lowercase() == e)
    }

    pub(crate) fn upsert_user(&self, mut u: UserRecord) -> std::io::Result<()> {
        let _g = self.lock.lock().unwrap();
        let mut db: UsersDb = read_json_or_default(&self.paths.users_db).unwrap_or_default();
        u.updated_at_unix = now_unix();
        let mut replaced = false;
        for item in db.users.iter_mut() {
            if item.username == u.username {
                *item = u.clone();
                replaced = true;
                break;
            }
        }
        if !replaced {
            db.users.push(u);
        }
        let json = serde_json::to_string_pretty(&db)?;
        atomic_write(&self.paths.users_db, json)
    }

    pub(crate) fn set_user_banned(&self, username: &str, banned: bool) -> std::io::Result<()> {
        let u = username.trim();
        if u.is_empty() {
            return Ok(());
        }
        let _g = self.lock.lock().unwrap();
        let mut db: UsersDb = read_json_or_default(&self.paths.users_db).unwrap_or_default();
        for item in db.users.iter_mut() {
            if item.username == u {
                item.banned = banned;
                item.updated_at_unix = now_unix();
                break;
            }
        }
        let json = serde_json::to_string_pretty(&db)?;
        atomic_write(&self.paths.users_db, json)?;
        // Drop all sessions for the user when banning.
        if banned {
            let mut sdb: SessionsDb = read_json_or_default(&self.paths.sessions_db).unwrap_or_default();
            sdb.sessions.retain(|s| s.username.as_deref() != Some(u));
            let sjson = serde_json::to_string_pretty(&sdb)?;
            atomic_write(&self.paths.sessions_db, sjson)?;
        }
        Ok(())
    }

    pub(crate) fn delete_user(&self, username: &str) -> std::io::Result<()> {
        let u = username.trim();
        if u.is_empty() {
            return Ok(());
        }
        let _g = self.lock.lock().unwrap();
        let mut db: UsersDb = read_json_or_default(&self.paths.users_db).unwrap_or_default();
        db.users.retain(|x| x.username != u);
        let json = serde_json::to_string_pretty(&db)?;
        atomic_write(&self.paths.users_db, json)?;

        let mut sdb: SessionsDb = read_json_or_default(&self.paths.sessions_db).unwrap_or_default();
        sdb.sessions.retain(|s| s.username.as_deref() != Some(u));
        let sjson = serde_json::to_string_pretty(&sdb)?;
        atomic_write(&self.paths.sessions_db, sjson)?;

        // Best-effort delete user data directory.
        let user_root = self.paths.per_user_root_dir.join(u);
        let _ = fs::remove_dir_all(user_root);
        Ok(())
    }

    pub(crate) fn create_user(
        &self,
        username: &str,
        nickname: &str,
        email: &str,
        role: AccountRole,
        password: Option<String>,
        login_option: LoginOption,
    ) -> Result<UserRecord, String> {
        let u = username.trim();
        if u.is_empty() {
            return Err("username is empty".to_string());
        }
        if self.find_user_by_username(u).is_some() {
            return Err("username already exists".to_string());
        }
        let e = email.trim().to_lowercase();
        if e.is_empty() {
            return Err("email is empty".to_string());
        }
        if self.find_user_by_email(&e).is_some() {
            return Err("email already exists".to_string());
        }
        let n = nickname.trim();
        if n.is_empty() {
            return Err("nickname is empty".to_string());
        }
        // Prevent confusing impersonation.
        if nickname_reserved(n) {
            return Err("nickname not allowed".to_string());
        }
        let now = now_unix();
        let rec = UserRecord {
            username: u.to_string(),
            nickname: n.to_string(),
            email: e,
            role,
            banned: false,
            login_option,
            password,
            created_at_unix: now,
            updated_at_unix: now,
            forced_notice: None,
            forced_notice_min_seconds: 0,
        };
        self.upsert_user(rec.clone()).map_err(|e| e.to_string())?;
        // Create user root dir for their data.
        let user_root = self.user_root_dir(&rec.username);
        fs::create_dir_all(&user_root).map_err(|e| e.to_string())?;
        Ok(rec)
    }

    pub(crate) fn user_root_dir(&self, username: &str) -> PathBuf {
        self.paths.per_user_root_dir.join(username)
    }

    pub(crate) fn create_session_for_user(&self, user: &UserRecord) -> Result<SessionRecord, String> {
        let _g = self.lock.lock().unwrap();
        let mut db: SessionsDb = read_json_or_default(&self.paths.sessions_db).unwrap_or_default();
        let now = now_unix();
        let expires = now.saturating_add(7 * 24 * 3600); // 7 days
        let session = SessionRecord {
            session_id: rand_id("sid", 32),
            username: Some(user.username.clone()),
            role: user.role,
            created_at_unix: now,
            expires_at_unix: expires,
        };
        db.sessions.push(session.clone());
        prune_sessions(&mut db, now);
        let json = serde_json::to_string_pretty(&db).map_err(|e| e.to_string())?;
        atomic_write(&self.paths.sessions_db, json).map_err(|e| e.to_string())?;
        Ok(session)
    }

    pub(crate) fn create_guest_session(&self) -> Result<SessionRecord, String> {
        let _g = self.lock.lock().unwrap();
        let mut db: SessionsDb = read_json_or_default(&self.paths.sessions_db).unwrap_or_default();
        let now = now_unix();
        let expires = now.saturating_add(24 * 3600); // 24h guest session
        let session = SessionRecord {
            session_id: rand_id("sidg", 32),
            username: None,
            role: AccountRole::Guest,
            created_at_unix: now,
            expires_at_unix: expires,
        };
        db.sessions.push(session.clone());
        prune_sessions(&mut db, now);
        let json = serde_json::to_string_pretty(&db).map_err(|e| e.to_string())?;
        atomic_write(&self.paths.sessions_db, json).map_err(|e| e.to_string())?;
        Ok(session)
    }

    pub(crate) fn delete_session(&self, session_id: &str) -> std::io::Result<()> {
        let sid = session_id.trim();
        if sid.is_empty() {
            return Ok(());
        }
        let _g = self.lock.lock().unwrap();
        let mut db: SessionsDb = read_json_or_default(&self.paths.sessions_db).unwrap_or_default();
        db.sessions.retain(|s| s.session_id != sid);
        let json = serde_json::to_string_pretty(&db)?;
        atomic_write(&self.paths.sessions_db, json)
    }

    pub(crate) fn resolve_session(&self, session_id: &str) -> Option<SessionRecord> {
        let sid = session_id.trim();
        if sid.is_empty() {
            return None;
        }
        let _g = self.lock.lock().unwrap();
        let now = now_unix();
        let db: SessionsDb = read_json_or_default(&self.paths.sessions_db).unwrap_or_default();
        db.sessions
            .into_iter()
            .find(|s| s.session_id == sid && s.expires_at_unix > now)
    }

    pub(crate) fn audit(&self, event: &str, fields: &HashMap<&str, String>) {
        let _g = self.lock.lock().unwrap();
        let now = now_unix();
        let mut obj = serde_json::Map::new();
        obj.insert("t".to_string(), serde_json::Value::from(now));
        obj.insert("event".to_string(), serde_json::Value::from(event));
        for (k, v) in fields {
            obj.insert((*k).to_string(), serde_json::Value::from(v.clone()));
        }
        let line = serde_json::Value::Object(obj).to_string();
        let _ = append_line(&self.paths.audit_log, &line);
    }
}

fn prune_sessions(db: &mut SessionsDb, now: u64) {
    db.sessions.retain(|s| s.expires_at_unix > now);
    // Hard cap to avoid unbounded growth.
    if db.sessions.len() > 2000 {
        db.sessions.sort_by_key(|s| s.created_at_unix);
        db.sessions.truncate(2000);
    }
}

fn nickname_reserved(nickname: &str) -> bool {
    let s = nickname.trim().to_lowercase();
    if s.is_empty() {
        return true;
    }
    // "or similar": block obvious admin-like nicknames.
    s == "admin" || s == "administrator" || s.contains("admin")
}

fn atomic_write(path: &Path, content: String) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{}", line)?;
    Ok(())
}

fn read_json_or_default<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Option<T> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str::<T>(&content).ok().or_else(|| Some(T::default()))
}

#[cfg(test)]
mod tests {
    use super::nickname_reserved;

    #[test]
    fn nickname_reservation_blocks_admin_like() {
        assert!(nickname_reserved("Admin"));
        assert!(nickname_reserved("administrator"));
        assert!(nickname_reserved("  aDmIn  "));
        assert!(nickname_reserved("my-admin-name"));
        assert!(!nickname_reserved("Kevin"));
    }
}
