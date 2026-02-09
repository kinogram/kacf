use anyhow::{anyhow, Context, Result};
use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::{deepseek_api, runner, workspace, git_utils};

/// Messages sent from the UI thread to the agent thread.
#[derive(Debug)]
pub enum AgentRequest {
    /// Start or continue a new session. Contains all user-configured fields.
    Start {
        api_key: String,
        base_url: String,
        model: String,
        workspace: PathBuf,
        goal: String,
        eval_cmd: String,
        success_regex: String,
    },
    /// Provide answers to clarification questions from the model.
    Clarify { answers: Vec<ClarifyAnswer> },
    /// Revert the most recent commit in the workspace via git.
    RevertLast,
    /// Stop the agent loop gracefully.
    Stop,
    /// Decision on whether to apply the most recent patch. When the agent
    /// sends a Diff event, the UI must send this to indicate whether to
    /// accept (`accept=true`) or reject (`accept=false`) the patch.
    ApplyPatch { accept: bool },
}

/// Messages sent from the agent thread back to the UI.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Append a line of text to the log.
    Log(String),
    /// The model needs clarification on some aspects before it can continue.
    NeedClarify { questions: Vec<ClarifyQuestion> },
    /// A unified diff of the most recent patch commit. Display this to the user.
    Diff { diff: String },
    /// The session has completed (successfully or not).
    Done { success: bool, message: String },
}

/// A question posed by the model for clarification. The `type` field indicates
/// the expected answer form: "single" (one of the options), "multi" (any
/// number of options), or "text" (free text input).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClarifyQuestion {
    pub id: String,
    pub question: String,
    #[serde(rename = "type")]
    pub qtype: String,
    #[serde(default)]
    pub options: Vec<String>,
}

/// A reply to a clarification question. Depending on the question type,
/// different fields will be used. For `single`, the `single` field should
/// contain the selected option. For `multi`, the `multi` vector should
/// contain all selected options. For `text`, the `text` field holds the
/// free-form response.
#[derive(Clone, Debug)]
pub struct ClarifyAnswer {
    pub id: String,
    pub qtype: String,
    pub single: String,
    pub multi: Vec<String>,
    pub text: String,
}

impl ClarifyAnswer {
    pub fn empty(id: &str, qtype: String) -> Self {
        Self {
            id: id.to_string(),
            qtype,
            single: String::new(),
            multi: vec![],
            text: String::new(),
        }
    }
}

/// Structured output returned by the model. The `kind` field determines
/// whether the model is asking clarification questions or providing a patch.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum ModelJson {
    #[serde(rename = "clarify")]
    Clarify { questions: Vec<ClarifyQuestion> },
    #[serde(rename = "patch")]
    Patch { summary: String, files: Vec<FileWrite> },
}

/// A file write instruction from the model. The `path` field is relative
/// to the workspace root and `content` holds the entire file contents.
#[derive(Debug, Serialize, Deserialize)]
pub struct FileWrite {
    pub path: String,
    pub content: String,
}

/// Run the agent loop. This function listens for requests from the UI and
/// interacts with the DeepSeek API, applying patches and evaluating the
/// resulting project. It sends events back to the UI to update state.
pub async fn agent_loop(rx_req: Receiver<AgentRequest>, tx_evt: Sender<AgentEvent>) {
    // Session state
    // stop_flag is passed by mutable reference into run_session so that a Stop
    // request from the UI can interrupt the session. It is also read at the
    // end of this function to avoid compiler warnings about unused
    // assignments.
    let mut stop_flag = false;
    let mut cfg: Option<SessionCfg> = None;
    let mut clarify_answers: Vec<ClarifyAnswer> = vec![];

    loop {
        match rx_req.recv() {
            Ok(AgentRequest::Start {
                api_key,
                base_url,
                model,
                workspace,
                goal,
                eval_cmd,
                success_regex,
            }) => {
                stop_flag = false;
                cfg = Some(SessionCfg {
                    api_key,
                    base_url,
                    model,
                    workspace,
                    goal,
                    eval_cmd,
                    success_regex,
                });
                clarify_answers.clear();
                let _ = tx_evt.send(AgentEvent::Log("[Agent] Start received".into()));
                if let Err(e) = run_session(cfg.as_ref().unwrap(), &mut clarify_answers, &rx_req, &tx_evt, &mut stop_flag).await {
                    let _ = tx_evt.send(AgentEvent::Done {
                        success: false,
                        message: format!("Session error: {:#}", e),
                    });
                }
            }
            Ok(AgentRequest::Clarify { answers }) => {
                clarify_answers = answers;
                let _ = tx_evt.send(AgentEvent::Log("[Agent] Clarify answers received".into()));
            }
            Ok(AgentRequest::RevertLast) => {
                // Attempt to revert the last commit via git.
                if let Some(ref cfg) = cfg {
                    if let Err(e) = git_utils::revert_last_commit(&cfg.workspace) {
                        let _ = tx_evt.send(AgentEvent::Log(format!("[Agent] 回滚失败: {e}")));
                    } else {
                        let _ = tx_evt.send(AgentEvent::Log("[Agent] 已回滚上一次提交".into()));
                    }
                }
            }
            Ok(AgentRequest::Stop) => {
                stop_flag = true;
                let _ = tx_evt.send(AgentEvent::Log("[Agent] Stop requested".into()));
                if cfg.is_some() {
                    let _ = tx_evt.send(AgentEvent::Done {
                        success: false,
                        message: "Stopped by user".into(),
                    });
                }
            }
            Ok(AgentRequest::ApplyPatch { .. }) => {
                // Patch decisions are handled within run_session. No action needed here.
            }
            Err(_) => break,
        }
    }

    // Read stop_flag to avoid unused assignment warnings. This has no
    // functional effect but ensures the compiler treats the variable as used.
    let _ = stop_flag;
}

#[derive(Clone)]
struct SessionCfg {
    api_key: String,
    base_url: String,
    model: String,
    workspace: PathBuf,
    goal: String,
    eval_cmd: String,
    success_regex: String,
}

/// The core loop for a single coding session. It communicates with the DeepSeek API
/// to generate patches, applies them to the workspace, commits the changes,
/// and evaluates the code. Iteration continues until success or a stop flag.
async fn run_session(
    cfg: &SessionCfg,
    clarify_answers: &mut Vec<ClarifyAnswer>,
    rx_req: &Receiver<AgentRequest>,
    tx_evt: &Sender<AgentEvent>,
    stop_flag: &mut bool,
) -> Result<()> {
    workspace::ensure_dir(&cfg.workspace)?;
    // Ensure a git repository is initialized so we can commit diffs.
    git_utils::init_repo_if_needed(&cfg.workspace)?;
    let _ = tx_evt.send(AgentEvent::Log(format!(
        "[Agent] workspace = {}",
        cfg.workspace.display()
    )));

    // Maintain a short conversation memory: system prompt plus previous
    // interactions. Each iteration we append model and user messages.
    let mut messages: Vec<deepseek_api::ChatMessage> = vec![];
    messages.push(deepseek_api::ChatMessage::system(system_prompt()));
    // Ask for clarification on the goal before generating any code.
    messages.push(deepseek_api::ChatMessage::user(format!(
        "大需求如下：\n{}\n\n请先输出 JSON：{{\"kind\":\"clarify\", ...}}，提出你认为会影响实现的疑义/选项（尽量少但关键）。如果你认为无需澄清，也输出 kind=patch，直接给出最小可运行实现。",
        cfg.goal
    )));

    'outer: for iter in 1..=30 {
        if *stop_flag {
            return Ok(());
        }
        let _ = tx_evt.send(AgentEvent::Log(format!("\n[Loop] Iteration {}", iter)));
        // Send the chat completion request to DeepSeek.
        let resp = deepseek_api::chat_complete(
            &cfg.base_url,
            &cfg.api_key,
            &cfg.model,
            &messages,
        )
        .await
        .context("deepseek chat_complete")?;
        let raw = resp.trim().to_string();
        let _ = tx_evt.send(AgentEvent::Log(format!(
            "[Model] raw content:\n{}",
            truncate(&raw, 2000)
        )));
        // 尝试提取可解析的 JSON 部分，并校验格式。
        let content = match extract_json(&raw) {
            Ok(json_str) => json_str,
            Err(e) => {
                // 如果 JSON 无法修复，则将错误反馈给模型以便其重新发送。
                messages.push(deepseek_api::ChatMessage::assistant(raw));
                messages.push(deepseek_api::ChatMessage::user(format!(
                    "你的回复无法解析为合法 JSON：{}。请严格按照约定输出 JSON 对象。",
                    e
                )));
                continue;
            }
        };
        let parsed: ModelJson = match serde_json::from_str(&content) {
            Ok(p) => p,
            Err(e) => {
                // 结构不合法，要求模型重新发送。
                messages.push(deepseek_api::ChatMessage::assistant(raw));
                messages.push(deepseek_api::ChatMessage::user(format!(
                    "JSON 解析错误：{}。请严格按照约定输出 JSON 对象。",
                    e
                )));
                continue;
            }
        };
        match parsed {
            ModelJson::Clarify { questions } => {
                // Ask the UI for clarification answers.
                let _ = tx_evt.send(AgentEvent::NeedClarify { questions });
                // Wait until we receive Clarify or Stop from UI.
                loop {
                    if *stop_flag {
                        return Ok(());
                    }
                    match rx_req.recv() {
                        Ok(AgentRequest::Clarify { answers }) => {
                            *clarify_answers = answers;
                            break;
                        }
                        Ok(AgentRequest::RevertLast) => {
                            // Handle revert request during clarification stage.
                            git_utils::revert_last_commit(&cfg.workspace).ok();
                            let _ = tx_evt.send(AgentEvent::Log("[Agent] 已回滚上一次提交".into()));
                        }
                        Ok(AgentRequest::Stop) => {
                            *stop_flag = true;
                            return Ok(());
                        }
                        _ => {}
                    }
                }
                // Send answers back to model and ask for a patch.
                let answers_json = serde_json::to_string_pretty(&to_answer_map(clarify_answers))?;
                messages.push(deepseek_api::ChatMessage::assistant(content));
                messages.push(deepseek_api::ChatMessage::user(format!(
                    "这是用户对澄清问题的回答(JSON)：\n{}\n\n现在请输出 kind=patch 的 JSON，给出最小可运行实现。要求：\n- 只输出 JSON（不要 markdown）\n- files 是完整文件内容（覆盖写）\n- 如果需要新文件，请直接提供。\n- 尽量小步提交，方便迭代。",
                    answers_json
                )));
            }
            ModelJson::Patch { summary, files } => {
                // 记录 patch 输出方便之后放入对话历史
                let patch_content = content.clone();
                let _ = tx_evt.send(AgentEvent::Log(format!("[Patch] {summary}")));
                // Write files to workspace.
                for f in files {
                    workspace::write_file_safely(&cfg.workspace, &f.path, &f.content)?;
                    let _ = tx_evt.send(AgentEvent::Log(format!(
                        "[Write] {} ({} bytes)",
                        f.path,
                        f.content.len()
                    )));
                }
                // Commit the patch so we can generate diffs and revert easily.
                let commit_message = format!("{summary}");
                if let Err(e) = git_utils::commit_all(&cfg.workspace, &commit_message) {
                    let _ = tx_evt.send(AgentEvent::Log(format!("[Agent] commit failed: {e}")));
                }
                // Generate diff for the last commit and send to UI. Even if diff
                // command fails the UI can ignore it gracefully.
                match git_utils::diff_last_commit(&cfg.workspace) {
                    Ok(diff) => {
                        let diff_truncated = truncate(&diff, 8000);
                        let _ = tx_evt.send(AgentEvent::Diff { diff: diff_truncated });
                    }
                    Err(e) => {
                        let _ = tx_evt.send(AgentEvent::Log(format!("[Agent] diff failed: {e}")));
                    }
                }
                // 等待用户对该补丁的决策：接受或拒绝。
                loop {
                    if *stop_flag {
                        return Ok(());
                    }
                    match rx_req.recv() {
                        Ok(AgentRequest::ApplyPatch { accept }) => {
                            if accept {
                                // 用户接受补丁，继续评测
                                let result = runner::run_eval(&cfg.workspace, &cfg.eval_cmd)?;
                                let _ = tx_evt.send(AgentEvent::Log(format!(
                                    "[Eval] exit={} \nstdout:\n{}\nstderr:\n{}",
                                    result.exit_code,
                                    truncate(&result.stdout, 2000),
                                    truncate(&result.stderr, 2000)
                                )));
                                let ok = result.exit_code == 0
                                    && (cfg.success_regex.trim().is_empty()
                                        || runner::regex_match(&cfg.success_regex, &(result.stdout.clone() + "\n" + &result.stderr)));
                                if ok {
                                    let _ = tx_evt.send(AgentEvent::Done {
                                        success: true,
                                        message: "目标达成：评测命令成功".into(),
                                    });
                                    return Ok(());
                                }
                                // 模型需要修复：将日志反馈给模型
                                messages.push(deepseek_api::ChatMessage::assistant(patch_content));
                                messages.push(deepseek_api::ChatMessage::user(format!(
                                    "本地评测失败。以下是执行日志，请你基于日志修复。\n\n命令：{}\nexit={}\n\nstdout:\n{}\n\nstderr:\n{}\n\n请继续输出 kind=patch JSON（只输出 JSON），用最小改动修复问题。",
                                    cfg.eval_cmd,
                                    result.exit_code,
                                    truncate(&result.stdout, 8000),
                                    truncate(&result.stderr, 8000)
                                )));
                                // 跳出等待循环进入下一轮迭代
                                break;
                            } else {
                                // 用户拒绝补丁：回滚最后提交并告知模型
                                let _ = git_utils::revert_last_commit(&cfg.workspace);
                                let _ = tx_evt.send(AgentEvent::Log("[Agent] 用户拒绝补丁，已回滚".into()));
                                // 将该补丁记入对话历史并指示模型重新生成
                                messages.push(deepseek_api::ChatMessage::assistant(patch_content));
                                messages.push(deepseek_api::ChatMessage::user("用户拒绝了此补丁，请根据需求重新生成新的 patch JSON。".to_string()));
                                // 直接进入下一轮
                                continue 'outer;
                            }
                        }
                        Ok(AgentRequest::RevertLast) => {
                            // 用户要求回滚：回滚并继续等待决定
                            let _ = git_utils::revert_last_commit(&cfg.workspace);
                            let _ = tx_evt.send(AgentEvent::Log("[Agent] 已回滚上一次提交".into()));
                        }
                        Ok(AgentRequest::Stop) => {
                            *stop_flag = true;
                            return Ok(());
                        }
                        Ok(AgentRequest::Clarify { .. }) => {
                            // 忽略 Clarify 消息：暂不适用
                        }
                        Err(_) | Ok(AgentRequest::Start { .. }) => {
                            // 无视其他消息
                        }
                    }
                }
            }
        }
    }
    let _ = tx_evt.send(AgentEvent::Done {
        success: false,
        message: "达到最大迭代次数仍未收敛".into(),
    });
    Ok(())
}

/// Build the system prompt for the DeepSeek model. This prompt constrains
/// the model to output strictly JSON in two allowed formats: either
/// clarification questions or a patch with files. It uses a numbered list
/// of rules to improve reliability.
fn system_prompt() -> String {
    r#"你是一个“自编程代理”。你必须严格遵守：
1) 你每次回复只能输出一个 JSON 对象，禁止输出 Markdown、解释、代码块围栏、自然语言。
2) JSON 只能是两种之一：
   A) {"kind":"clarify","questions":[{"id":"q1","question":"...","type":"single|multi|text","options":[".."]}]}
   B) {"kind":"patch","summary":"...","files":[{"path":"relative/path","content":"FULL FILE CONTENT"}]}
3) files.content 必须是完整文件内容（覆盖写），path 必须是相对路径，不允许写出工作区。
4) 小步提交：优先最小可运行版本，再迭代修复。
"#
        .to_string()
}

/// Convert a list of `ClarifyAnswer` into a JSON object mapping question ids to
/// their answers. This helper is used when returning answers to the model.
fn to_answer_map(answers: &[ClarifyAnswer]) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    for a in answers {
        let v = match a.qtype.as_str() {
            "single" => serde_json::Value::String(a.single.clone()),
            "multi" => serde_json::Value::Array(
                a.multi
                    .iter()
                    .cloned()
                    .map(serde_json::Value::String)
                    .collect(),
            ),
            "text" => serde_json::Value::String(a.text.clone()),
            _ => serde_json::Value::Null,
        };
        m.insert(a.id.clone(), v);
    }
    serde_json::Value::Object(m)
}

/// Truncate a string to a maximum length for log display. If the string is
/// longer than `max`, an ellipsis and truncation indicator are appended.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    format!("{}...\n[truncated {} chars]", &s[..max], s.len() - max)
}

/// 尝试从模型回复中提取可解析的 JSON 字符串。
/// 如果输入本身已经是合法 JSON，则直接返回。
/// 否则查找首个 '{' 与最后一个 '}' 之间的子串尝试解析。
fn extract_json(input: &str) -> Result<String> {
    // 如果整个字符串就是合法 JSON
    if serde_json::from_str::<serde_json::Value>(input).is_ok() {
        return Ok(input.to_string());
    }
    // 尝试提取第一个 '{' 到最后一个 '}' 之间的内容
    if let (Some(start), Some(end)) = (input.find('{'), input.rfind('}')) {
        if start < end {
            let candidate = &input[start..=end];
            if serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
                return Ok(candidate.to_string());
            }
        }
    }
    Err(anyhow!("未找到合法 JSON 段"))
}