use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use crate::auth::types::AdminSettings;

fn build_message(
    settings: &AdminSettings,
    to: &str,
    subject: &str,
    body: &str,
) -> Result<Message, String> {
    let from_raw = settings.smtp_from.trim();
    let from: Mailbox = from_raw
        .parse()
        .map_err(|e| format!("invalid smtp_from: {e}"))?;
    let to_box: Mailbox = to
        .parse()
        .map_err(|e| format!("invalid recipient email: {e}"))?;
    Message::builder()
        .from(from)
        .to(to_box)
        .subject(subject)
        .body(body.to_string())
        .map_err(|e| format!("build message failed: {e}"))
}

fn build_mailer(settings: &AdminSettings) -> Result<AsyncSmtpTransport<Tokio1Executor>, String> {
    let host = settings.smtp_host.trim();
    if host.trim().is_empty() {
        return Err("smtp_host is required when SMTP is enabled".to_string());
    }
    let port = settings.smtp_port;
    let username = settings.smtp_username.trim().to_string();
    let password = settings.smtp_password.clone();
    let use_starttls = settings.smtp_starttls;
    let mut builder = if use_starttls {
        AsyncSmtpTransport::<Tokio1Executor>::relay(host)
            .map_err(|e| format!("smtp relay init failed: {e}"))?
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
    };
    builder = builder.port(port);
    if !username.trim().is_empty() {
        builder = builder.credentials(Credentials::new(username, password));
    }
    Ok(builder.build())
}

pub(crate) async fn send_verification_code(
    settings: &AdminSettings,
    to: &str,
    scenario: &str,
    code: &str,
    ttl_secs: u64,
) -> Result<(), String> {
    if !settings.smtp_enabled {
        return Ok(());
    }
    let subject = "KACF verification code";
    let body = format!(
        "Scenario: {scenario}\nCode: {code}\nExpires in: {ttl_secs} seconds\n\nIf this was not requested by you, ignore this email."
    );
    let msg = build_message(settings, to, subject, &body)?;
    let mailer = build_mailer(settings)?;
    mailer
        .send(msg)
        .await
        .map_err(|e| format!("smtp send failed: {e}"))?;
    Ok(())
}
