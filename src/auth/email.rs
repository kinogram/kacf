use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

fn env_truthy(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(v) => {
            let t = v.trim().to_ascii_lowercase();
            t == "1" || t == "true" || t == "yes" || t == "on"
        }
        Err(_) => default,
    }
}

fn env_u16(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u16>().ok())
        .unwrap_or(default)
}

fn smtp_enabled() -> bool {
    env_truthy("AUTOCODING_SMTP_ENABLED", false)
}

fn build_message(to: &str, subject: &str, body: &str) -> Result<Message, String> {
    let from_raw = std::env::var("AUTOCODING_SMTP_FROM")
        .unwrap_or_else(|_| "noreply@localhost".to_string());
    let from: Mailbox = from_raw
        .parse()
        .map_err(|e| format!("invalid AUTOCODING_SMTP_FROM: {e}"))?;
    let to_box: Mailbox = to
        .parse()
        .map_err(|e| format!("invalid recipient email: {e}"))?;
    Message::builder()
        .from(from)
        .to(to_box)
        .subject(subject)
        .body(body.to_string())
        .map_err(|e| format!("build smtp message failed: {e}"))
}

fn build_mailer() -> Result<AsyncSmtpTransport<Tokio1Executor>, String> {
    let host = std::env::var("AUTOCODING_SMTP_HOST").unwrap_or_default();
    if host.trim().is_empty() {
        return Err("AUTOCODING_SMTP_HOST is required when SMTP is enabled".to_string());
    }
    let port = env_u16("AUTOCODING_SMTP_PORT", 587);
    let username = std::env::var("AUTOCODING_SMTP_USERNAME").unwrap_or_default();
    let password = std::env::var("AUTOCODING_SMTP_PASSWORD").unwrap_or_default();
    let use_starttls = env_truthy("AUTOCODING_SMTP_STARTTLS", true);
    let mut builder = if use_starttls {
        AsyncSmtpTransport::<Tokio1Executor>::relay(host.trim())
            .map_err(|e| format!("smtp relay builder failed: {e}"))?
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host.trim())
    };
    builder = builder.port(port);
    if !username.trim().is_empty() {
        builder = builder.credentials(Credentials::new(username, password));
    }
    Ok(builder.build())
}

pub(crate) async fn send_verification_code(
    to: &str,
    scenario: &str,
    code: &str,
    ttl_secs: u64,
) -> Result<(), String> {
    if !smtp_enabled() {
        return Ok(());
    }
    let subject = "KACF verification code";
    let body = format!(
        "Scenario: {scenario}\nCode: {code}\nExpires in: {ttl_secs} seconds\n\nIf this was not requested by you, ignore this email."
    );
    let msg = build_message(to, subject, &body)?;
    let mailer = build_mailer()?;
    mailer
        .send(msg)
        .await
        .map_err(|e| format!("smtp send failed: {e}"))?;
    Ok(())
}
