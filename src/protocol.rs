use anyhow::{anyhow, Context, Result};
use anyhow::Error;
use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::{deepseek_api, runner, workspace, git_utils};
use std::fs;

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
    /// Push the current git branch to a remote. Contains the remote name,
    /// remote URL, and the branch name. The agent will add or update the
    /// remote and then push the branch. Errors will be logged via events.
    PushRemote { remote: String, url: String, branch: String },
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
            Ok(AgentRequest::PushRemote { remote, url, branch }) => {
                // When receiving a push request, attempt to add/update the remote and push the branch.
                if let Some(ref cfg) = cfg {
                    let res: Result<(), Error> = (|| {
                        git_utils::add_remote(&cfg.workspace, &remote, &url)?;
                        git_utils::push(&cfg.workspace, &remote, &branch)?;
                        Ok(())
                    })();
                    match res {
                        Ok(_) => {
                            let _ = tx_evt.send(AgentEvent::Log(format!(
                                "[Agent] 已推送到远程 {} 的 {} 分支", remote, branch
                            )));
                        }
                        Err(e) => {
                            let _ = tx_evt.send(AgentEvent::Log(format!(
                                "[Agent] 推送远程失败: {:#}", e
                            )));
                        }
                    }
                }
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

/// Persisted session state for resuming an interrupted coding session. This
/// state is saved to a JSON file in the workspace directory after each
/// iteration. When starting a new session, if a matching state file is
/// present and its `goal` matches the current goal, the message history
/// stored here will be loaded so that the agent can resume conversations
/// with the model from the previous state. Only the message history and
/// goal are persisted; other runtime state (e.g. clarify answers) is
/// reconstructed at runtime.
#[derive(Serialize, Deserialize)]
struct SessionState {
    goal: String,
    messages: Vec<deepseek_api::ChatMessage>,
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

    // 尝试从工作目录加载先前保存的会话状态。如果存在且与当前目标一致，则继续该对话；否则开始新对话。
    let mut messages: Vec<deepseek_api::ChatMessage>;
    if let Some(state) = load_session_state(&cfg.workspace) {
        if state.goal == cfg.goal {
            messages = state.messages.clone();
            let _ = tx_evt.send(AgentEvent::Log("[Agent] 已加载之前的会话状态，继续从断点开始...".into()));
        } else {
            messages = Vec::new();
        }
    } else {
        messages = Vec::new();
    }
    // 如果没有历史消息，则初始化系统 prompt 和首条用户消息。
    if messages.is_empty() {
        messages.push(deepseek_api::ChatMessage::system(system_prompt()));
        messages.push(deepseek_api::ChatMessage::user(format!(
            "用户的需求如下：\n{}\n\n你需要作为自编程代理，根据该需求制定项目计划和目标，选择合适的技术栈并创建项目目录结构，编写代码，编译运行程序，分析并修复错误，如此往复循环，直至项目满足需求。为此，你应在项目目录中维护一个自动评测脚本（如 scripts/run_tests.sh），该脚本必须能够编译并运行程序、自动操作程序以执行必要的功能，并检测是否存在错误或未满足的目标。每次生成补丁后，你都需要更新这个评测脚本以反映新的需求。\n首先，请输出 JSON（kind=clarify 或 kind=patch）：如需澄清问题，请用 kind=clarify，并提出关键问题；如无需澄清，请用 kind=patch，并给出包含完整文件内容的补丁，补丁可以先生成项目计划、评测脚本或基本代码使项目能够编译运行。",
            cfg.goal
        )));
    }

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
                        Ok(AgentRequest::PushRemote { remote, url, branch }) => {
                            // Handle push remote during clarification stage. Attempt to add/update the
                            // remote and push the branch. Log any errors.
                            let res: Result<(), anyhow::Error> = (|| {
                                git_utils::add_remote(&cfg.workspace, &remote, &url)?;
                                git_utils::push(&cfg.workspace, &remote, &branch)?;
                                Ok(())
                            })();
                            match res {
                                Ok(_) => {
                                    let _ = tx_evt.send(AgentEvent::Log(format!(
                                        "[Agent] 已推送到远程 {} 的 {} 分支", remote, branch
                                    )));
                                }
                                Err(e) => {
                                    let _ = tx_evt.send(AgentEvent::Log(format!(
                                        "[Agent] 推送远程失败: {:#}", e
                                    )));
                                }
                            }
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
                // Save session state after appending new messages so that we can resume later.
                save_session_state(&cfg.workspace, &SessionState { goal: cfg.goal.clone(), messages: messages.clone() });
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
                // 自动评测补丁，无需用户确认。
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
                    // 评测成功：询问用户是否满意，允许其提出改进意见。
                    let feedback_question = ClarifyQuestion {
                        id: "feedback".to_string(),
                        question: "目标功能已实现，是否满意？如有改进意见请说明：".to_string(),
                        qtype: "text".to_string(),
                        options: vec![],
                    };
                    let _ = tx_evt.send(AgentEvent::NeedClarify {
                        questions: vec![feedback_question],
                    });
                    // 等待用户反馈
                    loop {
                        if *stop_flag {
                            return Ok(());
                        }
                        match rx_req.recv() {
                            Ok(AgentRequest::Clarify { answers }) => {
                                // 提取反馈文本（text 字段优先，其次 single，或空）
                                let mut feedback = String::new();
                                for ans in &answers {
                                    if ans.id == "feedback" {
                                        if !ans.text.is_empty() {
                                            feedback = ans.text.clone();
                                        } else if !ans.single.is_empty() {
                                            feedback = ans.single.clone();
                                        } else if !ans.multi.is_empty() {
                                            feedback = ans.multi.join(", ");
                                        }
                                        break;
                                    }
                                }
                                let trimmed = feedback.trim();
                                // 如果用户表示满意（空或含满意字样），结束会话；否则作为改进建议继续迭代。
                                if trimmed.is_empty() || trimmed.contains("满意") {
                                    // 保存当前会话状态并结束。将来如果重新启动，用户需重新设定目标。
                                    save_session_state(&cfg.workspace, &SessionState { goal: cfg.goal.clone(), messages: messages.clone() });
                                    let _ = tx_evt.send(AgentEvent::Done {
                                        success: true,
                                        message: "用户满意，项目完成".into(),
                                    });
                                    return Ok(());
                                } else {
                                    // 将用户反馈添加到对话历史，要求模型根据反馈改进代码。
                                    messages.push(deepseek_api::ChatMessage::assistant(patch_content.clone()));
                                    messages.push(deepseek_api::ChatMessage::user(format!(
                                        "用户反馈：{}。请根据反馈改进代码，生成新的 patch JSON（严格按照协议输出 JSON）。",
                                        trimmed
                                    )));
                                    // 保存状态以便断点续跑
                                    save_session_state(&cfg.workspace, &SessionState { goal: cfg.goal.clone(), messages: messages.clone() });
                                    // 跳出等待，进入下一轮迭代
                                    continue 'outer;
                                }
                            }
                            Ok(AgentRequest::RevertLast) => {
                                // 用户要求回滚：回滚最后提交并继续等待反馈
                                let _ = git_utils::revert_last_commit(&cfg.workspace);
                                let _ = tx_evt.send(AgentEvent::Log("[Agent] 已回滚上一次提交".into()));
                            }
                            Ok(AgentRequest::Stop) => {
                                *stop_flag = true;
                                return Ok(());
                            }
                            Ok(AgentRequest::PushRemote { remote, url, branch }) => {
                                // Handle push remote during feedback stage. Attempt to push and log.
                                let res: Result<(), anyhow::Error> = (|| {
                                    git_utils::add_remote(&cfg.workspace, &remote, &url)?;
                                    git_utils::push(&cfg.workspace, &remote, &branch)?;
                                    Ok(())
                                })();
                                match res {
                                    Ok(_) => {
                                        let _ = tx_evt.send(AgentEvent::Log(format!(
                                            "[Agent] 已推送到远程 {} 的 {} 分支", remote, branch
                                        )));
                                    }
                                    Err(e) => {
                                        let _ = tx_evt.send(AgentEvent::Log(format!(
                                            "[Agent] 推送远程失败: {:#}", e
                                        )));
                                    }
                                }
                            }
                            Err(_) | Ok(AgentRequest::Start { .. }) | Ok(AgentRequest::ApplyPatch { .. }) => {
                                // 忽略其他消息
                            }
                        }
                    }
                } else {
                    // 评测失败：将日志反馈给模型并继续迭代
                    messages.push(deepseek_api::ChatMessage::assistant(patch_content));
                    messages.push(deepseek_api::ChatMessage::user(format!(
                        "本地评测失败。以下是执行日志，请你基于日志修复。\n\n命令：{}\nexit={}\n\nstdout:\n{}\n\nstderr:\n{}\n\n请继续输出 kind=patch JSON（只输出 JSON），用最小改动修复问题。",
                        cfg.eval_cmd,
                        result.exit_code,
                        truncate(&result.stdout, 8000),
                        truncate(&result.stderr, 8000)
                    )));
                    // 保存状态后继续下一轮迭代
                    save_session_state(&cfg.workspace, &SessionState { goal: cfg.goal.clone(), messages: messages.clone() });
                    continue;
                }
            }
        }
    }
    // 达到最大迭代次数后保存状态，以便可能的续跑。
    save_session_state(&cfg.workspace, &SessionState { goal: cfg.goal.clone(), messages: messages.clone() });
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
    // 新的系统提示用以指导模型完成端到端的自编程流程。
    //
    // 本代理的职责是：用户仅提供高层需求，之后的所有工作（立项、制定项目目标和大纲、编写代码、编译运行、测试程序、分析和修复 bug、再次编译运行等）都由模型在闭环中完成。模型应循环执行：
    //  1. 生成或改进代码，并提供一个包含完整文件内容的补丁（patch）。每个补丁必须包含文件路径和完整内容。
    //  2. 运行用户指定的评测命令。若运行失败（编译错误、测试失败或程序运行错误），模型应根据日志进行修复。
    //  3. 当评测成功时，模型应询问用户是否满意或有改进建议。若用户提出改进意见，则根据建议继续迭代；若用户满意，则结束会话。
    // 模型可以在需要更多信息时提出澄清问题（clarify）。所有回复必须是严格的 JSON 对象，禁止任何 Markdown 或代码块围栏。
    // 允许输出的 JSON 只能有两种形式：
    //  A) {"kind": "clarify", "questions": [{"id": "q1", "question": "...", "type": "single|multi|text", "options": ["..."]}]}
    //     用于向用户提出疑问或选项。模型应尽量减少问题数量，确保问题关键且明确。
    //  B) {"kind": "patch", "summary": "...", "files": [{"path": "relative/path", "content": "FULL FILE CONTENT"}]}
    //     用于提供一个或多个文件的完整内容。summary 应说明补丁的目的，例如生成项目计划、编写某个模块代码、修复某个错误等。
    // 其他规则：
    //  - files.content 必须是完整文件内容（覆盖写入），不允许包含差异或补丁格式。
    //  - path 必须是相对路径，且写入位置只能位于工作目录内，禁止写出工作区。
    //  - 请采用小步迭代的方式：优先生成最小可运行版本，再逐步完善。
    r#"你是一个“自编程代理”。你必须严格遵守：
1) 你每次回复只能输出一个 JSON 对象，禁止输出 Markdown、解释、代码块围栏、自然语言。
2) 允许输出的 JSON 只能有两种形式：
   A) {"kind":"clarify","questions":[{"id":"q1","question":"...","type":"single|multi|text","options":["..."]}]}
      用于向用户提出疑问或选项，问题应尽量关键、简洁。
   B) {"kind":"patch","summary":"...","files":[{"path":"relative/path","content":"FULL FILE CONTENT"}]}
      用于生成或修改代码文件，summary 用中文说明补丁目的。
3) files.content 必须是完整文件内容（覆盖写入），path 必须是相对路径，不允许写出工作区。
4) 模型应在必要时创建或更新一个自动评测脚本（如 scripts/run_tests.sh），该脚本必须包含：编译项目、运行程序、按照项目要求自动操作程序、检测是否存在错误或未满足目标的情况。脚本应返回非零退出码以指示失败，并提供足够的日志供模型分析。
5) 按照“小步迭代”原则生成补丁：先生成最小可运行版本，再逐步完善和修复 bug。
6) 当评测脚本通过后，请在 summary 中提示已完成目标，并在下一轮中询问用户是否满意、是否有改进建议。若用户给出改进意见，请根据建议继续迭代。
7) 本项目提供了一个可执行的鼠标键盘自动化工具 `input_cli`（位于本仓库的二进制目标中）。你可以通过命令 `cargo run --release --features input-automation --bin input_cli -- <subcommand> [args...]` 调用它。支持的子命令有：
   - `move --x <int> --y <int>`：将鼠标移动到屏幕坐标 `(x, y)`。
   - `click --x <int> --y <int>`：将鼠标移动到 `(x, y)` 并点击左键。
   - `double-click --x <int> --y <int>`：移动并双击左键。
   - `right-click --x <int> --y <int>`：移动并右键点击。
   - `drag --from-x <int> --from-y <int> --to-x <int> --to-y <int>`：按住左键并从起点拖动到终点。
   - `hold --x <int> --y <int> --ms <int>`：在 `(x, y)` 坐标按住左键 `ms` 毫秒后释放，用于长按。
   - `type --text <string>`：在当前光标位置输入文本。
   - `keypress --key <key>`：按下并释放一个键，例如 enter、backspace、tab 或单个字符。
   - `shortcut --keys <k1> <k2> ...`：同时按下并释放多组组合键，例如 `--keys ctrl s` 对应 Ctrl+S。
   - `sleep --ms <int>`：暂停指定毫秒数。
   - `screenshot --output <path>`：捕获当前屏幕图像并保存为 PNG 文件（该功能在部分平台可能不可用）。该图像可供用户或后续分析使用，但模型本身无法直接读取图像。
   评测脚本可以调用这些命令来自动操作你的程序的用户界面，以查找并复现 bug。生成自动化脚本时，请根据程序窗口中的元素坐标、操作顺序调用这些命令，实现完整的交互测试。由于模型无法直接读取屏幕图像，而且截图功能仅在支持的操作系统上可用，它主要供用户审查或外部分析使用。
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

/// Attempt to load a persisted session state from the workspace. Returns
/// None if the file does not exist or cannot be parsed. The state file
/// name is hard-coded as `.autocoding_state.json` in the workspace root.
fn load_session_state(workspace: &std::path::Path) -> Option<SessionState> {
    let state_path = workspace.join(".autocoding_state.json");
    match fs::read_to_string(&state_path) {
        Ok(content) => serde_json::from_str::<SessionState>(&content).ok(),
        Err(_) => None,
    }
}

/// Save the current session state to the workspace. Errors during saving
/// are ignored (logged to stderr) but do not interrupt the session.
fn save_session_state(workspace: &std::path::Path, state: &SessionState) {
    let state_path = workspace.join(".autocoding_state.json");
    if let Ok(json) = serde_json::to_string_pretty(state) {
        // Write to a temporary file then rename for atomicity.
        let tmp_path = state_path.with_extension("tmp");
        if fs::write(&tmp_path, json).is_ok() {
            let _ = fs::rename(tmp_path, state_path);
        }
    }
}
