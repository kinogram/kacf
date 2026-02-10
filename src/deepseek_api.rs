use anyhow::{anyhow, Context, Result};
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
    if api_key.trim().is_empty() {
        return Err(anyhow!(
            "API key is empty. 请先在界面填写 DeepSeek API Key（或设置环境变量 DEEPSEEK_API_KEY）。"
        ));
    }
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
        .with_context(|| format!("POST {}", url))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let hint = match status.as_u16() {
            401 => "401 Unauthorized：API Key 无效/过期，或与当前 Base URL 不匹配（这不是频率限制）。",
            429 => "429 Too Many Requests：请求过于频繁或额度限制，请稍后重试。",
            _ => "请求失败，请检查 API Key、Base URL、模型名和网络环境。",
        };
        let body = truncate_for_log(&body, 400);
        return Err(anyhow!(
            "HTTP status not success: {} for url ({}). {} response={}",
            status,
            url,
            hint,
            body
        ));
    }

    let resp = serde_json::from_str::<ChatCompletionResp>(&body).context("parse json response")?;
    let content = resp
        .choices
        .get(0)
        .and_then(|c| c.message.content.clone())
        .unwrap_or_default();
    Ok(content)
}

fn truncate_for_log(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    format!("{}...[truncated {} chars]", &s[..max], s.len() - max)
}
