use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Represents a single chat message for the DeepSeek API. The `role` field
/// may be "system", "user", or "assistant", and `content` holds the
/// message text.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(s: String) -> Self {
        Self {
            role: "system".into(),
            content: s,
        }
    }
    pub fn user(s: String) -> Self {
        Self {
            role: "user".into(),
            content: s,
        }
    }
    pub fn assistant(s: String) -> Self {
        Self {
            role: "assistant".into(),
            content: s,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResp {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChoiceMessage {
    content: Option<String>,
}

#[derive(Debug, Serialize)]
struct ChatCompletionReq<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    stream: bool,
    // Additional optional fields (temperature, top_p, etc.) could be added here.
}

/// Call the DeepSeek chat completion API. This function performs an HTTP
/// POST to the configured base URL and returns the content of the first
/// returned message. Errors during the request or JSON parsing are
/// propagated. The API key is sent via the Authorization header.
pub async fn chat_complete(
    base_url: &str,
    api_key: &str,
    model: &str,
    messages: &[ChatMessage],
) -> Result<String> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let req = ChatCompletionReq {
        model,
        messages,
        stream: false,
    };
    let resp = client
        .post(&url)
        .bearer_auth(api_key)
        .json(&req)
        .send()
        .await
        .with_context(|| format!("POST {}", url))?
        .error_for_status()
        .context("HTTP status not success")?
        .json::<ChatCompletionResp>()
        .await
        .context("parse json response")?;
    let content = resp
        .choices
        .get(0)
        .and_then(|c| c.message.content.clone())
        .unwrap_or_default();
    Ok(content)
}