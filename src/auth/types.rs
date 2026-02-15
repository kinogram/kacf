use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AccountRole {
    Admin,
    User,
    Guest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LoginOption {
    PasswordOnly,
    PasswordEmail2fa,
    EmailOnly,
    // Reserved for future:
    // PasswordTotp2fa,
    // TotpOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct UserRecord {
    pub(crate) username: String,
    pub(crate) nickname: String,
    pub(crate) email: String,
    pub(crate) role: AccountRole,
    pub(crate) banned: bool,
    pub(crate) login_option: LoginOption,
    /// Plaintext password by requirement (admin can view passwords).
    pub(crate) password: Option<String>,
    pub(crate) created_at_unix: u64,
    pub(crate) updated_at_unix: u64,
    #[serde(default)]
    pub(crate) forced_notice: Option<String>,
    #[serde(default)]
    pub(crate) forced_notice_min_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AdminSettings {
    #[serde(default = "default_true")]
    pub(crate) registration_enabled: bool,
    #[serde(default = "default_true")]
    pub(crate) guest_enabled: bool,
    #[serde(default = "default_true")]
    pub(crate) login_enabled: bool,
    #[serde(default = "default_true")]
    pub(crate) non_admin_login_enabled: bool,
}

impl Default for AdminSettings {
    fn default() -> Self {
        Self {
            registration_enabled: true,
            guest_enabled: true,
            login_enabled: true,
            non_admin_login_enabled: true,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SessionRecord {
    pub(crate) session_id: String,
    pub(crate) username: Option<String>,
    pub(crate) role: AccountRole,
    pub(crate) created_at_unix: u64,
    pub(crate) expires_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AuthMeResponse {
    pub(crate) logged_in: bool,
    pub(crate) role: AccountRole,
    pub(crate) username: Option<String>,
    pub(crate) nickname: String,
    #[serde(default)]
    pub(crate) email: Option<String>,
    pub(crate) is_admin: bool,
    pub(crate) guest: bool,
    pub(crate) banned: bool,
    #[serde(default)]
    pub(crate) forced_notice: Option<String>,
    #[serde(default)]
    pub(crate) forced_notice_min_seconds: u32,
}
