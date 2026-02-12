use anyhow::Error;
use anyhow::{anyhow, Context, Result};
use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use crate::{deepseek_api, git_utils, runner, workspace};
use std::fs;

thread_local! {
    static CURRENT_AUTO_REVERT_PROFILE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Messages sent from the UI thread to the agent thread.
#[derive(Debug)]
pub enum AgentRequest {
    /// Start or continue a new session. Contains all user-configured fields.
    Start {
        api_key: String,
        base_url: String,
        model: String,
        auto_revert_profile: String,
        resume_from_checkpoint: bool,
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
    /// Push the current git branch to a remote. Contains the remote name,
    /// remote URL, and the branch name. The agent will add or update the
    /// remote and then push the branch. Errors will be logged via events.
    PushRemote {
        remote: String,
        url: String,
        branch: String,
    },
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

/// Structured output returned by the model. The `kind` field determines
/// whether the model is asking clarification questions or providing a patch.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum ModelJson {
    #[serde(rename = "clarify")]
    Clarify { questions: Vec<ClarifyQuestion> },
    #[serde(rename = "patch")]
    Patch {
        summary: String,
        files: Vec<FileWrite>,
    },
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
                auto_revert_profile,
                resume_from_checkpoint,
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
                    auto_revert_profile,
                    resume_from_checkpoint,
                    workspace,
                    goal,
                    eval_cmd,
                    success_regex,
                });
                clarify_answers.clear();
                let _ = tx_evt.send(AgentEvent::Log("[Agent] Start received".into()));
                if let Err(e) = run_session(
                    cfg.as_ref().unwrap(),
                    &mut clarify_answers,
                    &rx_req,
                    &tx_evt,
                    &mut stop_flag,
                )
                .await
                {
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
            Ok(AgentRequest::PushRemote {
                remote,
                url,
                branch,
            }) => {
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
                                "[Agent] 已推送到远程 {} 的 {} 分支",
                                remote, branch
                            )));
                        }
                        Err(e) => {
                            let _ = tx_evt
                                .send(AgentEvent::Log(format!("[Agent] 推送远程失败: {:#}", e)));
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
    auto_revert_profile: String,
    resume_from_checkpoint: bool,
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
    #[serde(default = "default_schema_version")]
    schema_version: u32,
    goal: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    eval_cmd: String,
    #[serde(default)]
    iteration: u32,
    #[serde(default)]
    last_status: String,
    #[serde(default)]
    updated_at_unix: u64,
    #[serde(default)]
    message_count: usize,
    #[serde(default)]
    messages: Vec<deepseek_api::ChatMessage>,
}

fn default_schema_version() -> u32 {
    2
}

#[derive(Default)]
struct RepairHeuristics {
    total_patches: u32,
    consecutive_eval_failures: u32,
    consecutive_same_failure: u32,
    last_failure_signature: String,
    last_failure_severity: u8,
    last_failure_category: String,
    last_auto_revert_iter: u32,
}

#[derive(Clone)]
struct EvalPipelineReport {
    result: runner::EvalResult,
    stage: String,
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
    CURRENT_AUTO_REVERT_PROFILE.with(|v| {
        *v.borrow_mut() = Some(cfg.auto_revert_profile.to_lowercase());
    });
    workspace::ensure_dir(&cfg.workspace)?;
    // Ensure a git repository is initialized so we can commit diffs.
    git_utils::init_repo_if_needed(&cfg.workspace)?;
    let _ = tx_evt.send(AgentEvent::Log(format!(
        "[Agent] workspace = {}",
        cfg.workspace.display()
    )));

    // 尝试从工作目录加载先前保存的会话状态。如果存在且与当前目标一致，则继续该对话；否则开始新对话。
    let mut messages: Vec<deepseek_api::ChatMessage>;
    if cfg.resume_from_checkpoint {
        if let Some(state) = load_session_state(&cfg.workspace) {
            if state.goal == cfg.goal {
                messages = state.messages.clone();
                let _ = tx_evt.send(AgentEvent::Log(
                    "[Agent] 已加载之前的会话状态，继续从断点开始...".into(),
                ));
            } else {
                messages = Vec::new();
            }
        } else {
            messages = Vec::new();
        }
    } else {
        messages = Vec::new();
        let _ = tx_evt.send(AgentEvent::Log(
            "[Agent] 本次按新任务启动，已忽略历史会话状态".into(),
        ));
    }
    // 如果没有历史消息，则初始化系统 prompt 和首条用户消息。
    if messages.is_empty() {
        messages.push(deepseek_api::ChatMessage::system(system_prompt()));
        messages.push(deepseek_api::ChatMessage::user(format!(
            "用户的需求如下：\n{}\n\n你需要作为自编程代理，根据该需求制定项目计划和目标，选择合适的技术栈并创建项目目录结构，编写代码，编译运行程序，分析并修复错误，如此往复循环，直至项目满足需求。为此，你应在项目目录中维护一个自动评测脚本（如 scripts/run_tests.sh），该脚本必须能够编译并运行程序、自动操作程序以执行必要的功能，并检测是否存在错误或未满足的目标。每次生成补丁后，你都需要更新这个评测脚本以反映新的需求。\n首先，请输出 JSON（kind=clarify 或 kind=patch）：如需澄清问题，请用 kind=clarify，并提出关键问题；如无需澄清，请用 kind=patch，并给出包含完整文件内容的补丁，补丁可以先生成项目计划、评测脚本或基本代码使项目能够编译运行。",
            cfg.goal
        )));
    }

    // Save an initial checkpoint as soon as session context is ready.
    save_session_state(
        &cfg.workspace,
        &build_session_state(cfg, &messages, 0, "session_initialized"),
    );
    let mut repair = RepairHeuristics::default();

    'outer: for iter in 1..=30 {
        if *stop_flag {
            return Ok(());
        }
        if let Some((removed_msgs, removed_chars)) = trim_message_history_for_speed(&mut messages) {
            let _ = tx_evt.send(AgentEvent::Log(format!(
                "[Agent] 为提升性能已裁剪上下文: removed_messages={} removed_chars={}",
                removed_msgs, removed_chars
            )));
        }
        save_session_state(
            &cfg.workspace,
            &build_session_state(cfg, &messages, iter, "iteration_started"),
        );
        let _ = tx_evt.send(AgentEvent::Log(format!("\n[Loop] Iteration {}", iter)));
        let api_start = std::time::Instant::now();
        // Send the chat completion request to DeepSeek and keep emitting
        // progress logs so UI users can distinguish "still generating" from "stuck".
        let resp = chat_complete_with_progress(cfg, &messages, tx_evt)
            .await
            .context("deepseek chat_complete")?;
        let api_ms = api_start.elapsed().as_millis();
        let _ = tx_evt.send(AgentEvent::Log(format!("[Perf] deepseek_api={}ms", api_ms)));
        let raw = resp.trim().to_string();
        if raw.len() > 200_000 {
            messages.push(deepseek_api::ChatMessage::assistant(
                compact_assistant_text(&raw, 12_000),
            ));
            messages.push(deepseek_api::ChatMessage::user(
                "你的回复过大（超过 200KB），请仅输出必要 JSON 并显著缩短内容。".to_string(),
            ));
            continue;
        }
        let _ = tx_evt.send(AgentEvent::Log(format!(
            "[Model] raw content:\n{}",
            truncate(&raw, 2000)
        )));
        // 尝试提取可解析的 JSON 部分，并校验格式。
        let content = match extract_json(&raw) {
            Ok(json_str) => json_str,
            Err(e) => {
                // 如果 JSON 无法修复，则将错误反馈给模型以便其重新发送。
                messages.push(deepseek_api::ChatMessage::assistant(
                    compact_assistant_text(&raw, 12_000),
                ));
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
                messages.push(deepseek_api::ChatMessage::assistant(
                    compact_assistant_text(&raw, 12_000),
                ));
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
                        Ok(AgentRequest::PushRemote {
                            remote,
                            url,
                            branch,
                        }) => {
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
                                        "[Agent] 已推送到远程 {} 的 {} 分支",
                                        remote, branch
                                    )));
                                }
                                Err(e) => {
                                    let _ = tx_evt.send(AgentEvent::Log(format!(
                                        "[Agent] 推送远程失败: {:#}",
                                        e
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
                save_session_state(
                    &cfg.workspace,
                    &build_session_state(cfg, &messages, iter, "waiting_patch_after_clarify"),
                );
            }
            ModelJson::Patch { summary, files } => {
                // 记录 patch 输出方便之后放入对话历史
                let patch_content = content.clone();
                let patch_history = compact_patch_history(&summary, &files, &patch_content);
                repair.total_patches += 1;
                if let Err(e) = validate_patch_payload(&summary, &files) {
                    let _ =
                        tx_evt.send(AgentEvent::Log(format!("[Patch] 无效补丁，已拒绝：{}", e)));
                    messages.push(deepseek_api::ChatMessage::assistant(patch_content));
                    messages.push(deepseek_api::ChatMessage::user(format!(
                        "你给出的 patch 无效：{}。请重新输出 kind=patch JSON，只做必要最小改动，并确保 files 中每个 path 唯一且 content 是完整文件内容。",
                        e
                    )));
                    save_session_state(
                        &cfg.workspace,
                        &build_session_state(cfg, &messages, iter, "invalid_patch_payload"),
                    );
                    continue;
                }
                let total_bytes: usize = files.iter().map(|f| f.content.len()).sum();
                let _ = tx_evt.send(AgentEvent::Log(format!(
                    "[Patch] #{} {} | files={} total_bytes={}",
                    repair.total_patches,
                    summary,
                    files.len(),
                    total_bytes
                )));
                // Write files to workspace.
                for f in &files {
                    workspace::write_file_safely(&cfg.workspace, &f.path, &f.content)?;
                    let _ = tx_evt.send(AgentEvent::Log(format!(
                        "[Write] {} ({} bytes)",
                        f.path,
                        f.content.len()
                    )));
                }
                // Commit the patch so we can generate diffs and revert easily.
                let commit_message = summary.to_string();
                if let Err(e) = git_utils::commit_all(&cfg.workspace, &commit_message) {
                    let _ = tx_evt.send(AgentEvent::Log(format!("[Agent] commit failed: {e}")));
                }
                // Generate diff for the last commit and send to UI. Even if diff
                // command fails the UI can ignore it gracefully.
                let mut diff_for_feedback = String::new();
                match git_utils::diff_last_commit(&cfg.workspace) {
                    Ok(diff) => {
                        diff_for_feedback = truncate(&diff, 3000);
                        let diff_truncated = truncate(&diff, 8000);
                        let _ = tx_evt.send(AgentEvent::Diff {
                            diff: diff_truncated,
                        });
                    }
                    Err(e) => {
                        let _ = tx_evt.send(AgentEvent::Log(format!("[Agent] diff failed: {e}")));
                    }
                }
                // 评测流水线：可选预检 + 主评测。
                let eval_report = run_eval_pipeline(cfg, tx_evt)?;
                let result = eval_report.result;
                let _ = tx_evt.send(AgentEvent::Log(format!(
                    "[Eval:{}] exit={} \nstdout:\n{}\nstderr:\n{}",
                    eval_report.stage,
                    result.exit_code,
                    truncate(&result.stdout, 2000),
                    truncate(&result.stderr, 2000)
                )));
                let ok = result.exit_code == 0
                    && !eval_has_fatal_runtime_marker(&result.stdout, &result.stderr)
                    && (cfg.success_regex.trim().is_empty()
                        || runner::regex_match(
                            &cfg.success_regex,
                            &(result.stdout.clone() + "\n" + &result.stderr),
                        ));
                if result.exit_code == 0
                    && eval_has_fatal_runtime_marker(&result.stdout, &result.stderr)
                {
                    let _ = tx_evt.send(AgentEvent::Log(
                        "[Eval-Guard] exit=0 但检测到致命运行错误标记（如 panic/X11/display），按失败处理".to_string(),
                    ));
                }
                if ok {
                    repair.consecutive_eval_failures = 0;
                    repair.consecutive_same_failure = 0;
                    repair.last_failure_signature.clear();
                    repair.last_failure_severity = 0;
                    repair.last_failure_category.clear();
                    // 再做一次回归评测，降低“偶然通过”导致的假完成。
                    let verify_t0 = std::time::Instant::now();
                    let verify = runner::run_eval(&cfg.workspace, &cfg.eval_cmd)?;
                    let _ = tx_evt.send(AgentEvent::Log(format!(
                        "[Perf] eval_verify={}ms",
                        verify_t0.elapsed().as_millis()
                    )));
                    let verify_ok = verify.exit_code == 0
                        && !eval_has_fatal_runtime_marker(&verify.stdout, &verify.stderr)
                        && (cfg.success_regex.trim().is_empty()
                            || runner::regex_match(
                                &cfg.success_regex,
                                &(verify.stdout.clone() + "\n" + &verify.stderr),
                            ));
                    if verify.exit_code == 0
                        && eval_has_fatal_runtime_marker(&verify.stdout, &verify.stderr)
                    {
                        let _ = tx_evt.send(AgentEvent::Log(
                            "[Eval-Guard] verify exit=0 但检测到致命运行错误标记（如 panic/X11/display），按失败处理".to_string(),
                        ));
                    }
                    let _ = tx_evt.send(AgentEvent::Log(format!(
                        "[Eval-Verify] exit={} \nstdout:\n{}\nstderr:\n{}",
                        verify.exit_code,
                        truncate(&verify.stdout, 1200),
                        truncate(&verify.stderr, 1200)
                    )));
                    if !verify_ok {
                        let digest =
                            digest_eval_failure(verify.exit_code, &verify.stdout, &verify.stderr);
                        let severity = failure_severity(digest.category);
                        repair.consecutive_eval_failures = 1;
                        repair.consecutive_same_failure = 1;
                        repair.last_failure_signature = digest.signature.clone();
                        repair.last_failure_severity = severity;
                        repair.last_failure_category = digest.category.to_string();
                        messages.push(deepseek_api::ChatMessage::assistant(patch_history.clone()));
                        messages.push(deepseek_api::ChatMessage::user(format!(
                            "你上一次补丁在主评测通过，但在回归复测失败。请优先修复不稳定/漏测问题。\n{}",
                            build_repair_prompt(
                                cfg,
                                iter,
                                &verify,
                                &digest,
                                &repair,
                                &diff_for_feedback,
                            )
                        )));
                        save_session_state(
                            &cfg.workspace,
                            &build_session_state(cfg, &messages, iter, "eval_verify_failed"),
                        );
                        continue;
                    }
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
                                    save_session_state(
                                        &cfg.workspace,
                                        &build_session_state(
                                            cfg,
                                            &messages,
                                            iter,
                                            "done_user_satisfied",
                                        ),
                                    );
                                    let _ = tx_evt.send(AgentEvent::Done {
                                        success: true,
                                        message: "用户满意，项目完成".into(),
                                    });
                                    return Ok(());
                                } else {
                                    // 将用户反馈添加到对话历史，要求模型根据反馈改进代码。
                                    messages.push(deepseek_api::ChatMessage::assistant(
                                        patch_history.clone(),
                                    ));
                                    messages.push(deepseek_api::ChatMessage::user(format!(
                                        "用户反馈：{}。请根据反馈改进代码，生成新的 patch JSON（严格按照协议输出 JSON）。",
                                        trimmed
                                    )));
                                    // 保存状态以便断点续跑
                                    save_session_state(
                                        &cfg.workspace,
                                        &build_session_state(
                                            cfg,
                                            &messages,
                                            iter,
                                            "feedback_requires_improvement",
                                        ),
                                    );
                                    // 跳出等待，进入下一轮迭代
                                    continue 'outer;
                                }
                            }
                            Ok(AgentRequest::RevertLast) => {
                                // 用户要求回滚：回滚最后提交并继续等待反馈
                                let _ = git_utils::revert_last_commit(&cfg.workspace);
                                let _ =
                                    tx_evt.send(AgentEvent::Log("[Agent] 已回滚上一次提交".into()));
                            }
                            Ok(AgentRequest::Stop) => {
                                *stop_flag = true;
                                return Ok(());
                            }
                            Ok(AgentRequest::PushRemote {
                                remote,
                                url,
                                branch,
                            }) => {
                                // Handle push remote during feedback stage. Attempt to push and log.
                                let res: Result<(), anyhow::Error> = (|| {
                                    git_utils::add_remote(&cfg.workspace, &remote, &url)?;
                                    git_utils::push(&cfg.workspace, &remote, &branch)?;
                                    Ok(())
                                })(
                                );
                                match res {
                                    Ok(_) => {
                                        let _ = tx_evt.send(AgentEvent::Log(format!(
                                            "[Agent] 已推送到远程 {} 的 {} 分支",
                                            remote, branch
                                        )));
                                    }
                                    Err(e) => {
                                        let _ = tx_evt.send(AgentEvent::Log(format!(
                                            "[Agent] 推送远程失败: {:#}",
                                            e
                                        )));
                                    }
                                }
                            }
                            Err(_) | Ok(AgentRequest::Start { .. }) => {
                                // 忽略其他消息
                            }
                        }
                    }
                } else {
                    // 评测失败：构建结构化失败反馈并继续迭代
                    let digest =
                        digest_eval_failure(result.exit_code, &result.stdout, &result.stderr);
                    let severity = failure_severity(digest.category);
                    let previous_severity = repair.last_failure_severity;
                    let previous_category = repair.last_failure_category.clone();
                    let signature = digest.signature.clone();
                    repair.consecutive_eval_failures += 1;
                    if !signature.is_empty() && signature == repair.last_failure_signature {
                        repair.consecutive_same_failure += 1;
                    } else {
                        repair.consecutive_same_failure = 1;
                    }
                    repair.last_failure_signature = signature;
                    repair.last_failure_severity = severity;
                    repair.last_failure_category = digest.category.to_string();
                    let _ = tx_evt.send(AgentEvent::Log(format!(
                        "[Eval-Digest] category={} severity={} repeated={} signature={}",
                        digest.category,
                        severity,
                        repair.consecutive_same_failure,
                        digest.signature
                    )));
                    messages.push(deepseek_api::ChatMessage::assistant(patch_history));
                    messages.push(deepseek_api::ChatMessage::user(build_repair_prompt(
                        cfg,
                        iter,
                        &result,
                        &digest,
                        &repair,
                        &diff_for_feedback,
                    )));
                    if should_auto_revert(&repair, &digest, previous_severity, iter) {
                        let _ = git_utils::revert_last_commit(&cfg.workspace);
                        let _ = tx_evt.send(AgentEvent::Log(
                            "[Agent] 检测到连续重复错误，已自动回滚最近一次提交并要求小步修复"
                                .into(),
                        ));
                        messages.push(deepseek_api::ChatMessage::user(
                            "检测到你连续产生同一错误，系统已自动回滚最近一次补丁。请基于根因做更小步修复，避免重复失败。"
                                .to_string(),
                        ));
                        if category_changed_worse(&previous_category, digest.category) {
                            messages.push(deepseek_api::ChatMessage::user(
                                "错误类别较上一轮恶化，请先恢复到可编译/可运行状态，再进行功能迭代。"
                                    .to_string(),
                            ));
                        }
                        repair.last_auto_revert_iter = iter;
                    }
                    // 保存状态后继续下一轮迭代
                    save_session_state(
                        &cfg.workspace,
                        &build_session_state(cfg, &messages, iter, "eval_failed_waiting_fix"),
                    );
                    continue;
                }
            }
        }
    }
    // 达到最大迭代次数后保存状态，以便可能的续跑。
    save_session_state(
        &cfg.workspace,
        &build_session_state(cfg, &messages, 30, "max_iterations_reached"),
    );
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
8) 当你收到“评测失败”反馈时，你必须优先分析失败根因，并针对关键错误行修复。禁止无关重构，禁止一次性改太多文件。
9) 如果同类错误连续出现，你必须优先修复测试脚本/构建脚本与入口参数的不一致问题，并在 patch 中显式更新对应脚本，防止同类错误再次出现。
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

fn trim_message_history_for_speed(
    messages: &mut Vec<deepseek_api::ChatMessage>,
) -> Option<(usize, usize)> {
    let max_messages = read_history_max_messages();
    let max_chars = read_history_max_chars();
    if messages.is_empty() {
        return None;
    }
    let total_chars: usize = messages.iter().map(|m| m.content.len()).sum();
    if messages.len() <= max_messages && total_chars <= max_chars {
        return None;
    }

    let mut kept = Vec::new();
    let mut used_chars = 0usize;
    let mut start_idx = 0usize;

    if messages[0].role == "system" {
        used_chars += messages[0].content.len();
        kept.push(messages[0].clone());
        start_idx = 1;
    }

    let mut tail = Vec::new();
    for idx in (start_idx..messages.len()).rev() {
        let m = &messages[idx];
        let next_count = kept.len() + tail.len() + 1;
        let next_chars = used_chars + m.content.len();
        if next_count > max_messages || next_chars > max_chars {
            break;
        }
        tail.push(m.clone());
        used_chars = next_chars;
    }
    tail.reverse();
    kept.extend(tail);

    let removed_msgs = messages.len().saturating_sub(kept.len());
    let removed_chars = total_chars.saturating_sub(used_chars);
    if removed_msgs == 0 {
        return None;
    }
    *messages = kept;
    Some((removed_msgs, removed_chars))
}

fn read_history_max_messages() -> usize {
    std::env::var("AUTOCODING_HISTORY_MAX_MESSAGES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v >= 10 && *v <= 200)
        .unwrap_or(40)
}

fn read_history_max_chars() -> usize {
    std::env::var("AUTOCODING_HISTORY_MAX_CHARS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v >= 10_000 && *v <= 2_000_000)
        .unwrap_or(70_000)
}

fn compact_assistant_text(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    format!(
        "{}\n...[history truncated {} chars]",
        &s[..max],
        s.len() - max
    )
}

fn compact_patch_history(summary: &str, files: &[FileWrite], raw_patch_json: &str) -> String {
    let paths = files
        .iter()
        .take(30)
        .map(|f| f.path.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let header = format!(
        "kind=patch summary={} file_count={} paths=[{}]",
        summary,
        files.len(),
        paths
    );
    let raw = compact_assistant_text(raw_patch_json, 16_000);
    format!("{header}\n{raw}")
}

fn run_eval_pipeline(cfg: &SessionCfg, tx_evt: &Sender<AgentEvent>) -> Result<EvalPipelineReport> {
    if let Some(precheck_cmd) = read_precheck_cmd() {
        let _ = tx_evt.send(AgentEvent::Log(format!(
            "[Eval-Precheck] 开始执行: {}",
            precheck_cmd
        )));
        let t0 = std::time::Instant::now();
        let pre = runner::run_eval(&cfg.workspace, &precheck_cmd)?;
        let _ = tx_evt.send(AgentEvent::Log(format!(
            "[Perf] eval_precheck={}ms",
            t0.elapsed().as_millis()
        )));
        let _ = tx_evt.send(AgentEvent::Log(format!(
            "[Eval-Precheck] cmd='{}' exit={}",
            precheck_cmd, pre.exit_code
        )));
        if pre.exit_code != 0 {
            return Ok(EvalPipelineReport {
                result: pre,
                stage: "precheck".to_string(),
            });
        }
    }
    let _ = tx_evt.send(AgentEvent::Log(format!(
        "[Eval-Main] 开始执行: {}",
        cfg.eval_cmd
    )));
    let t1 = std::time::Instant::now();
    let main = runner::run_eval(&cfg.workspace, &cfg.eval_cmd)?;
    let _ = tx_evt.send(AgentEvent::Log(format!(
        "[Perf] eval_main={}ms",
        t1.elapsed().as_millis()
    )));
    Ok(EvalPipelineReport {
        result: main,
        stage: "main".to_string(),
    })
}

async fn chat_complete_with_progress(
    cfg: &SessionCfg,
    messages: &[deepseek_api::ChatMessage],
    tx_evt: &Sender<AgentEvent>,
) -> Result<String> {
    let _ = tx_evt.send(AgentEvent::Log("[Model] 正在请求模型响应...".to_string()));
    let started_at = std::time::Instant::now();
    #[derive(Default)]
    struct PreviewState {
        buf: String,
        chars: usize,
        events: usize,
    }
    let preview = RefCell::new(PreviewState::default());
    let mut on_delta = |delta: &str| {
        if delta.is_empty() {
            return;
        }
        let mut st = preview.borrow_mut();
        st.chars += delta.chars().count();
        st.events += 1;
        st.buf.push_str(delta);
        let should_flush = st.buf.len() >= 120
            || delta.contains('\n')
            || delta.contains('}')
            || delta.contains(']');
        if should_flush {
            let out = st.buf.replace('\n', "\\n");
            st.buf.clear();
            let _ = tx_evt.send(AgentEvent::Log(format!(
                "[Model-Stream] {}",
                truncate(&out, 600)
            )));
        }
    };
    let req = deepseek_api::chat_complete_streaming(
        &cfg.base_url,
        &cfg.api_key,
        &cfg.model,
        messages,
        &mut on_delta,
    );
    tokio::pin!(req);
    let mut ticker = tokio::time::interval(Duration::from_secs(5));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut first_tick = true;
    loop {
        tokio::select! {
            out = &mut req => {
                let st = preview.borrow();
                if !st.buf.is_empty() {
                    let out_tail = st.buf.replace('\n', "\\n");
                    let _ = tx_evt.send(AgentEvent::Log(format!(
                        "[Model-Stream] {}",
                        truncate(&out_tail, 600)
                    )));
                }
                let _ = tx_evt.send(AgentEvent::Log(format!(
                    "[Model] 流式完成: chunks={} chars={}",
                    st.events, st.chars
                )));
                return out;
            },
            _ = ticker.tick() => {
                if first_tick {
                    first_tick = false;
                    continue;
                }
                let _ = tx_evt.send(AgentEvent::Log(format!(
                    "[Model] 仍在生成中... {}s",
                    started_at.elapsed().as_secs()
                )));
            }
        }
    }
}

fn read_precheck_cmd() -> Option<String> {
    let cmd = std::env::var("AUTOCODING_PRECHECK_CMD").ok()?;
    if cmd.trim().is_empty() {
        return None;
    }
    Some(cmd)
}

fn auto_revert_on_repeat() -> bool {
    bool_env("AUTOCODING_AUTO_REVERT_ON_REPEAT", false)
}

fn auto_revert_repeat_count() -> u32 {
    read_u32_env("AUTOCODING_AUTO_REVERT_REPEAT_COUNT", 2, 20).unwrap_or_else(|| {
        match auto_revert_profile().as_str() {
            "conservative" => 4,
            "aggressive" => 2,
            _ => 3,
        }
    })
}

fn auto_revert_min_severity() -> u8 {
    read_u8_env("AUTOCODING_AUTO_REVERT_MIN_SEVERITY", 0, 3).unwrap_or_else(|| {
        match auto_revert_profile().as_str() {
            "conservative" => 3,
            "aggressive" => 1,
            _ => 2,
        }
    })
}

fn auto_revert_on_worse() -> bool {
    bool_env("AUTOCODING_AUTO_REVERT_ON_WORSE", true)
}

fn auto_revert_min_worse_delta() -> u8 {
    read_u8_env("AUTOCODING_AUTO_REVERT_MIN_WORSE_DELTA", 0, 3).unwrap_or_else(|| {
        match auto_revert_profile().as_str() {
            "conservative" => 2,
            "aggressive" => 0,
            _ => 1,
        }
    })
}

fn auto_revert_cooldown_iters() -> u32 {
    read_u32_env("AUTOCODING_AUTO_REVERT_COOLDOWN_ITERS", 0, 20).unwrap_or_else(|| {
        match auto_revert_profile().as_str() {
            "conservative" => 3,
            "aggressive" => 1,
            _ => 2,
        }
    })
}

fn auto_revert_profile() -> String {
    if let Some(cfg_profile) = CURRENT_AUTO_REVERT_PROFILE.with(|v| v.borrow().clone()) {
        return cfg_profile;
    }
    std::env::var("AUTOCODING_AUTO_REVERT_PROFILE")
        .ok()
        .map(|v| v.to_lowercase())
        .filter(|v| matches!(v.as_str(), "conservative" | "balanced" | "aggressive"))
        .unwrap_or_else(|| "balanced".to_string())
}

fn read_u32_env(key: &str, min: u32, max: u32) -> Option<u32> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| *v >= min && *v <= max)
}

fn read_u8_env(key: &str, min: u8, max: u8) -> Option<u8> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<u8>().ok())
        .filter(|v| *v >= min && *v <= max)
}

fn bool_env(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(default)
}

fn failure_severity(category: &str) -> u8 {
    match category {
        "compile_error" => 3,
        "runtime_crash" => 3,
        "test_failure" => 2,
        "runtime_or_logic_failure" => 2,
        "timeout" => 1,
        _ => 1,
    }
}

fn category_changed_worse(previous: &str, current: &str) -> bool {
    failure_severity(current) > failure_severity(previous)
}

fn should_auto_revert(
    repair: &RepairHeuristics,
    digest: &FailureDigest,
    previous_severity: u8,
    iter: u32,
) -> bool {
    if !auto_revert_on_repeat() {
        return false;
    }
    let cooldown = auto_revert_cooldown_iters();
    if repair.last_auto_revert_iter > 0 && iter < repair.last_auto_revert_iter + cooldown {
        return false;
    }
    let repeated_limit = auto_revert_repeat_count();
    let severity_now = failure_severity(digest.category);
    let severity_gate = severity_now >= auto_revert_min_severity();
    let repeated_gate = repair.consecutive_same_failure >= repeated_limit;
    let min_delta = auto_revert_min_worse_delta();
    let worsened_gate =
        auto_revert_on_worse() && severity_now >= previous_severity.saturating_add(min_delta);
    let signature_gate = auto_revert_signature_allowed(&digest.signature);
    (repeated_gate || worsened_gate) && severity_gate && signature_gate
}

fn auto_revert_signature_allowed(signature: &str) -> bool {
    let sig = signature.to_lowercase();
    let deny = read_keyword_list("AUTOCODING_AUTO_REVERT_SIGNATURE_DENY");
    if !deny.is_empty() && deny.iter().any(|k| sig.contains(k)) {
        return false;
    }
    let allow = read_keyword_list("AUTOCODING_AUTO_REVERT_SIGNATURE_ALLOW");
    if allow.is_empty() {
        return true;
    }
    allow.iter().any(|k| sig.contains(k))
}

fn read_keyword_list(key: &str) -> Vec<String> {
    std::env::var(key)
        .ok()
        .map(|v| {
            v.split(',')
                .map(|x| x.trim().to_lowercase())
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn validate_patch_payload(summary: &str, files: &[FileWrite]) -> Result<()> {
    if summary.trim().is_empty() {
        return Err(anyhow!("summary 不能为空"));
    }
    if files.is_empty() {
        return Err(anyhow!("files 不能为空"));
    }
    if files.len() > 80 {
        return Err(anyhow!("单次 patch 文件数量过多: {}", files.len()));
    }
    let mut seen = HashSet::new();
    let mut total_bytes = 0usize;
    for f in files {
        let path = f.path.trim();
        if path.is_empty() {
            return Err(anyhow!("存在空 path"));
        }
        if !seen.insert(path.to_string()) {
            return Err(anyhow!("path 重复: {}", path));
        }
        if f.content.len() > 700_000 {
            return Err(anyhow!("单文件过大: {} ({} bytes)", path, f.content.len()));
        }
        total_bytes += f.content.len();
    }
    if total_bytes > 3_000_000 {
        return Err(anyhow!("本次 patch 总体积过大: {} bytes", total_bytes));
    }
    Ok(())
}

struct FailureDigest {
    category: &'static str,
    signature: String,
    key_lines: Vec<String>,
}

fn digest_eval_failure(exit_code: i32, stdout: &str, stderr: &str) -> FailureDigest {
    let joined = format!("{stdout}\n{stderr}");
    let lowered = joined.to_lowercase();
    let category =
        if exit_code == -2 || lowered.contains("timed out") || lowered.contains("timeout") {
            "timeout"
        } else if lowered.contains("error[") || lowered.contains("could not compile") {
            "compile_error"
        } else if lowered.contains("test result: failed")
            || lowered.contains("failures:")
            || lowered.contains("assertion failed")
        {
            "test_failure"
        } else if lowered.contains("panicked") || lowered.contains("traceback") {
            "runtime_crash"
        } else {
            "runtime_or_logic_failure"
        };

    let key_lines = collect_key_lines(stdout, stderr, 8);
    let signature = key_lines
        .first()
        .cloned()
        .unwrap_or_else(|| format!("exit_code={exit_code}"));

    FailureDigest {
        category,
        signature,
        key_lines,
    }
}

fn eval_has_fatal_runtime_marker(stdout: &str, stderr: &str) -> bool {
    let lowered = format!("{stdout}\n{stderr}").to_lowercase();
    lowered.contains("thread 'main' panicked")
        || lowered.contains(" panicked at ")
        || lowered.contains("xopendisplay() failed")
        || lowered.contains("segmentation fault")
        || lowered.contains("stack backtrace:")
        || lowered.contains("fatal runtime error")
}

fn collect_key_lines(stdout: &str, stderr: &str, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in stderr.lines() {
        maybe_add_key_line(&mut out, line, limit);
        if out.len() >= limit {
            return out;
        }
    }
    for line in stdout.lines() {
        maybe_add_key_line(&mut out, line, limit);
        if out.len() >= limit {
            return out;
        }
    }
    out
}

fn maybe_add_key_line(out: &mut Vec<String>, line: &str, limit: usize) {
    if out.len() >= limit {
        return;
    }
    let l = line.trim();
    if l.is_empty() || l.len() > 220 {
        return;
    }
    let lowered = l.to_lowercase();
    let hit = lowered.contains("error")
        || lowered.contains("failed")
        || lowered.contains("panic")
        || lowered.contains("assert")
        || lowered.contains("exception")
        || lowered.contains("traceback")
        || lowered.contains("timeout")
        || lowered.contains("could not compile")
        || lowered.contains("caused by");
    if hit && !out.iter().any(|x| x == l) {
        out.push(l.to_string());
    }
}

fn build_repair_prompt(
    cfg: &SessionCfg,
    iter: u32,
    result: &runner::EvalResult,
    digest: &FailureDigest,
    repair: &RepairHeuristics,
    diff_for_feedback: &str,
) -> String {
    let repeated_hint = if repair.consecutive_same_failure >= 2 {
        "检测到同类错误重复出现。你必须先写出根因，再给出最小修复，并补充/修正测试以防回归。"
    } else {
        "请基于错误日志做最小必要修复。"
    };
    let strictness_hint = if repair.consecutive_eval_failures >= 3 {
        "你需要额外检查：依赖版本、构建脚本、入口参数、测试脚本本身是否错误。"
    } else {
        ""
    };
    let key_lines_text = if digest.key_lines.is_empty() {
        "（无）".to_string()
    } else {
        digest
            .key_lines
            .iter()
            .map(|x| format!("- {x}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "本地评测失败，请继续修复。\n\
迭代轮次: {iter}\n\
命令: {cmd}\n\
exit_code: {exit}\n\
错误类别: {category}\n\
失败签名: {signature}\n\
连续失败次数: {fail_count}\n\
同签名重复次数: {same_count}\n\
\n\
关键错误行:\n{key_lines}\n\
\n\
最近补丁 diff 摘要:\n{diff}\n\
\n\
stdout:\n{stdout}\n\
\n\
stderr:\n{stderr}\n\
\n\
修复要求:\n\
1) {repeated}\n\
2) 先修复导致失败的直接原因，再处理次要问题。\n\
3) 如果评测脚本本身不准确，先修正评测脚本再修代码。\n\
4) 输出必须是 kind=patch JSON，且只输出 JSON。\n\
5) 小步修改，避免大面积重写。\n\
6) 修复后必须确保 `{cmd}` 可通过。{strictness}",
        cmd = cfg.eval_cmd,
        exit = result.exit_code,
        category = digest.category,
        signature = digest.signature,
        fail_count = repair.consecutive_eval_failures,
        same_count = repair.consecutive_same_failure,
        key_lines = key_lines_text,
        diff = if diff_for_feedback.is_empty() {
            "（当前 diff 不可用）"
        } else {
            diff_for_feedback
        },
        stdout = truncate(&result.stdout, 8000),
        stderr = truncate(&result.stderr, 8000),
        repeated = repeated_hint,
        strictness = strictness_hint,
    )
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

fn build_session_state(
    cfg: &SessionCfg,
    messages: &[deepseek_api::ChatMessage],
    iteration: u32,
    status: &str,
) -> SessionState {
    SessionState {
        schema_version: 2,
        goal: cfg.goal.clone(),
        model: cfg.model.clone(),
        eval_cmd: cfg.eval_cmd.clone(),
        iteration,
        last_status: status.to_string(),
        updated_at_unix: now_unix(),
        message_count: messages.len(),
        messages: messages.to_vec(),
    }
}

fn now_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        auto_revert_min_severity, auto_revert_on_repeat, auto_revert_repeat_count,
        auto_revert_signature_allowed, category_changed_worse, compact_assistant_text,
        failure_severity, trim_message_history_for_speed, validate_patch_payload, FileWrite,
    };
    use crate::deepseek_api::ChatMessage;

    #[test]
    fn trim_history_keeps_system_and_recent_messages() {
        let mut msgs = vec![ChatMessage::system("system".to_string())];
        for i in 0..80 {
            msgs.push(ChatMessage::user(format!("user-{i}-{}", "x".repeat(400))));
        }
        let removed = trim_message_history_for_speed(&mut msgs).expect("should trim");
        assert!(removed.0 > 0);
        assert_eq!(msgs.first().map(|m| m.role.as_str()), Some("system"));
        assert!(msgs.len() <= 40);
    }

    #[test]
    fn validate_patch_rejects_duplicate_paths() {
        let files = vec![
            FileWrite {
                path: "a.txt".to_string(),
                content: "1".to_string(),
            },
            FileWrite {
                path: "a.txt".to_string(),
                content: "2".to_string(),
            },
        ];
        let err = validate_patch_payload("summary", &files).expect_err("must fail");
        assert!(err.to_string().contains("重复"));
    }

    #[test]
    fn compact_text_truncates_large_history() {
        let s = "a".repeat(100);
        let c = compact_assistant_text(&s, 32);
        assert!(c.len() < s.len() + 64);
        assert!(c.contains("truncated"));
    }

    #[test]
    fn auto_revert_flag_parses_boolean_values() {
        std::env::set_var("AUTOCODING_AUTO_REVERT_ON_REPEAT", "true");
        assert!(auto_revert_on_repeat());
        std::env::set_var("AUTOCODING_AUTO_REVERT_ON_REPEAT", "0");
        assert!(!auto_revert_on_repeat());
        std::env::remove_var("AUTOCODING_AUTO_REVERT_ON_REPEAT");
        assert!(!auto_revert_on_repeat());
    }

    #[test]
    fn severity_mapping_is_reasonable() {
        assert!(failure_severity("compile_error") > failure_severity("timeout"));
        assert!(failure_severity("runtime_crash") >= failure_severity("test_failure"));
    }

    #[test]
    fn category_worse_detection_works() {
        assert!(category_changed_worse("timeout", "compile_error"));
        assert!(!category_changed_worse("compile_error", "test_failure"));
    }

    #[test]
    fn signature_allow_deny_works() {
        std::env::set_var("AUTOCODING_AUTO_REVERT_SIGNATURE_ALLOW", "panic,compile");
        std::env::set_var("AUTOCODING_AUTO_REVERT_SIGNATURE_DENY", "timeout");
        assert!(auto_revert_signature_allowed("compile failed at crate x"));
        assert!(!auto_revert_signature_allowed("timeout while waiting"));
        assert!(!auto_revert_signature_allowed("network error"));
        std::env::remove_var("AUTOCODING_AUTO_REVERT_SIGNATURE_ALLOW");
        std::env::remove_var("AUTOCODING_AUTO_REVERT_SIGNATURE_DENY");
    }

    #[test]
    fn auto_revert_profile_changes_defaults() {
        std::env::remove_var("AUTOCODING_AUTO_REVERT_REPEAT_COUNT");
        std::env::remove_var("AUTOCODING_AUTO_REVERT_MIN_SEVERITY");
        std::env::set_var("AUTOCODING_AUTO_REVERT_PROFILE", "aggressive");
        assert_eq!(auto_revert_repeat_count(), 2);
        assert_eq!(auto_revert_min_severity(), 1);
        std::env::set_var("AUTOCODING_AUTO_REVERT_PROFILE", "conservative");
        assert_eq!(auto_revert_repeat_count(), 4);
        assert_eq!(auto_revert_min_severity(), 3);
        std::env::remove_var("AUTOCODING_AUTO_REVERT_PROFILE");
    }
}
