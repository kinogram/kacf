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

impl LoginOption {
    pub(crate) fn requires_password(&self) -> bool {
        matches!(
            self,
            LoginOption::PasswordOnly | LoginOption::PasswordEmail2fa
        )
    }

    pub(crate) fn requires_email_code(&self) -> bool {
        matches!(self, LoginOption::EmailOnly | LoginOption::PasswordEmail2fa)
    }
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
    #[serde(default)]
    pub(crate) email_code_dev_mode: bool,
    #[serde(default = "default_email_code_ttl_secs")]
    pub(crate) email_code_ttl_secs: u64,
    #[serde(default = "default_email_code_resend_cooldown_secs")]
    pub(crate) email_code_resend_cooldown_secs: u64,
    #[serde(default = "default_email_issue_per_min")]
    pub(crate) email_issue_per_min: u32,
    #[serde(default = "default_email_verify_per_min")]
    pub(crate) email_verify_per_min: u32,
    #[serde(default = "default_true")]
    pub(crate) email_domain_allowlist_enabled: bool,
    #[serde(default = "default_email_domain_allowlist")]
    pub(crate) email_domain_allowlist: Vec<String>,
    #[serde(default)]
    pub(crate) smtp_enabled: bool,
    #[serde(default)]
    pub(crate) smtp_host: String,
    #[serde(default = "default_smtp_port")]
    pub(crate) smtp_port: u16,
    #[serde(default)]
    pub(crate) smtp_username: String,
    #[serde(default)]
    pub(crate) smtp_password: String,
    #[serde(default = "default_smtp_from")]
    pub(crate) smtp_from: String,
    #[serde(default = "default_true")]
    pub(crate) smtp_starttls: bool,
}

impl Default for AdminSettings {
    fn default() -> Self {
        Self {
            registration_enabled: true,
            guest_enabled: true,
            login_enabled: true,
            non_admin_login_enabled: true,
            email_code_dev_mode: false,
            email_code_ttl_secs: default_email_code_ttl_secs(),
            email_code_resend_cooldown_secs: default_email_code_resend_cooldown_secs(),
            email_issue_per_min: default_email_issue_per_min(),
            email_verify_per_min: default_email_verify_per_min(),
            email_domain_allowlist_enabled: true,
            email_domain_allowlist: default_email_domain_allowlist(),
            smtp_enabled: false,
            smtp_host: String::new(),
            smtp_port: default_smtp_port(),
            smtp_username: String::new(),
            smtp_password: String::new(),
            smtp_from: default_smtp_from(),
            smtp_starttls: true,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_email_code_ttl_secs() -> u64 {
    600
}

fn default_email_code_resend_cooldown_secs() -> u64 {
    60
}

fn default_email_issue_per_min() -> u32 {
    3
}

fn default_email_verify_per_min() -> u32 {
    12
}

fn default_email_domain_allowlist() -> Vec<String> {
    vec![
        "gmail.com".to_string(),
        "outlook.com".to_string(),
        "hotmail.com".to_string(),
        "live.com".to_string(),
        "yahoo.com".to_string(),
        "icloud.com".to_string(),
        "proton.me".to_string(),
        "qq.com".to_string(),
        "163.com".to_string(),
        "126.com".to_string(),
        "foxmail.com".to_string(),
    ]
}

fn default_smtp_port() -> u16 {
    587
}

fn default_smtp_from() -> String {
    "noreply@localhost".to_string()
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
