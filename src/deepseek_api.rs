use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;

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

#[derive(Debug, Serialize)]
struct ChatCompletionReq<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    stream: bool,
    // Additional optional fields (temperature, top_p, etc.) could be added here.
}

#[derive(Debug, Deserialize)]
struct StreamChunkResp {
    choices: Option<Vec<StreamChoice>>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: Option<StreamDelta>,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    content: Option<String>,
    reasoning_content: Option<String>,
}

/// Call DeepSeek chat completion in streaming mode (`stream=true`) and feed
/// each content delta into `on_delta`. The final merged assistant content is
/// returned as a single String.
pub async fn chat_complete_streaming(
    base_url: &str,
    api_key: &str,
    model: &str,
    messages: &[ChatMessage],
    on_delta: &mut dyn FnMut(&str),
) -> Result<String> {
    let mut sink = |_delta: &str| {};
    chat_complete_streaming_with_reasoning(base_url, api_key, model, messages, on_delta, &mut sink)
        .await
}

pub async fn chat_complete_streaming_with_reasoning(
    base_url: &str,
    api_key: &str,
    model: &str,
    messages: &[ChatMessage],
    on_delta: &mut dyn FnMut(&str),
    on_reasoning: &mut dyn FnMut(&str),
) -> Result<String> {
    if api_key.trim().is_empty() {
        return Err(anyhow!(
            "API key is empty. 请先在界面填写 DeepSeek API Key（或设置环境变量 DEEPSEEK_API_KEY）。"
        ));
    }
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let client = shared_client();
    let req = ChatCompletionReq {
        model,
        messages,
        stream: true,
    };
    let max_attempts = 3u32;
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 1..=max_attempts {
        let send_result = client
            .post(&url)
            .bearer_auth(api_key)
            .json(&req)
            .send()
            .await;
        match send_result {
            Ok(resp) => {
                let status = resp.status();
                if !status.is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    let hint = status_hint(status.as_u16());
                    let body_short = truncate_for_log(&body, 400);
                    let err = anyhow!(
                        "HTTP status not success: {} for url ({}). {} response={}",
                        status,
                        url,
                        hint,
                        body_short
                    );
                    if is_retryable_status(status.as_u16()) && attempt < max_attempts {
                        tokio::time::sleep(retry_delay(attempt)).await;
                        last_err = Some(err);
                        continue;
                    }
                    return Err(err);
                }
                match parse_streaming_response(resp, on_delta, on_reasoning).await {
                    Ok(text) => return Ok(text),
                    Err(e) => {
                        if attempt < max_attempts {
                            tokio::time::sleep(retry_delay(attempt)).await;
                            last_err = Some(e);
                            continue;
                        }
                        return Err(e);
                    }
                }
            }
            Err(e) => {
                let err = anyhow!(e).context(format!("POST {}", url));
                if is_retryable_transport_error(&err) && attempt < max_attempts {
                    tokio::time::sleep(retry_delay(attempt)).await;
                    last_err = Some(err);
                    continue;
                }
                return Err(err);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("chat completion streaming failed")))
}

async fn parse_streaming_response(
    mut resp: reqwest::Response,
    on_delta: &mut dyn FnMut(&str),
    on_reasoning: &mut dyn FnMut(&str),
) -> Result<String> {
    let mut line_buf = String::new();
    let mut merged = String::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| anyhow!(e).context("read streaming chunk"))?
    {
        let text = std::str::from_utf8(&chunk).map_err(|e| anyhow!(e).context("utf8 chunk"))?;
        line_buf.push_str(text);
        while let Some(idx) = line_buf.find('\n') {
            let mut line = line_buf[..idx].to_string();
            line_buf.drain(..=idx);
            if line.ends_with('\r') {
                line.pop();
            }
            handle_sse_line(&line, &mut merged, on_delta, on_reasoning)?;
        }
    }
    if !line_buf.trim().is_empty() {
        handle_sse_line(line_buf.trim(), &mut merged, on_delta, on_reasoning)?;
    }
    if merged.trim().is_empty() {
        return Err(anyhow!("streaming response completed with empty content"));
    }
    Ok(merged)
}

fn handle_sse_line(
    line: &str,
    merged: &mut String,
    on_delta: &mut dyn FnMut(&str),
    on_reasoning: &mut dyn FnMut(&str),
) -> Result<()> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with(':') {
        return Ok(());
    }
    if !trimmed.starts_with("data:") {
        return Ok(());
    }
    let payload = trimmed.trim_start_matches("data:").trim();
    if payload.is_empty() || payload == "[DONE]" {
        return Ok(());
    }
    let parsed: StreamChunkResp = serde_json::from_str(payload).map_err(|e| {
        let short = truncate_for_log(payload, 300);
        anyhow!("parse streaming chunk failed: {} payload={}", e, short)
    })?;
    if let Some(choices) = parsed.choices {
        for choice in choices {
            if let Some(delta) = choice.delta {
                if let Some(content) = delta.content {
                    if !content.is_empty() {
                        merged.push_str(&content);
                        on_delta(&content);
                    }
                }
                if let Some(reasoning) = delta.reasoning_content {
                    if !reasoning.is_empty() {
                        on_reasoning(&reasoning);
                    }
                }
            }
        }
    }
    Ok(())
}

fn truncate_for_log(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    format!("{}...[truncated {} chars]", &s[..max], s.len() - max)
}

fn shared_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(read_timeout_secs()))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(8)
            .tcp_keepalive(Duration::from_secs(30))
            .build()
            .expect("build reqwest client")
    })
}

fn read_timeout_secs() -> u64 {
    std::env::var("AUTOCODING_API_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v >= 10 && *v <= 300)
        .unwrap_or(90)
}

fn retry_delay(attempt: u32) -> Duration {
    match attempt {
        1 => Duration::from_millis(350),
        2 => Duration::from_millis(900),
        _ => Duration::from_millis(1600),
    }
}

fn is_retryable_status(code: u16) -> bool {
    matches!(code, 408 | 409 | 425 | 429 | 500 | 502 | 503 | 504)
}

fn is_retryable_transport_error(err: &anyhow::Error) -> bool {
    let s = err.to_string().to_lowercase();
    s.contains("timed out")
        || s.contains("timeout")
        || s.contains("connection reset")
        || s.contains("connection refused")
        || s.contains("temporarily unavailable")
}

fn status_hint(code: u16) -> &'static str {
    match code {
        400 => "400 Bad Request：请求参数格式错误，请检查 model/messages 结构。",
        401 => "401 Unauthorized：API Key 无效/过期，或与当前 Base URL 不匹配。",
        403 => "403 Forbidden：当前 Key 无权限调用该模型或接口。",
        404 => "404 Not Found：Base URL 或接口路径错误。",
        408 => "408 Request Timeout：请求超时，将自动重试。",
        429 => "429 Too Many Requests：请求过于频繁或额度限制，将自动重试。",
        500 | 502 | 503 | 504 => "服务端暂时异常，将自动重试。",
        _ => "请求失败，请检查 API Key、Base URL、模型名和网络环境。",
    }
}
