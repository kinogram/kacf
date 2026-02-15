use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender};

use crate::protocol_auto_revert;
pub(crate) use crate::protocol_models::SessionState;
pub use crate::protocol_models::{AgentEvent, AgentRequest, ClarifyAnswer, ClarifyQuestion};
use crate::protocol_models::{EvalPipelineReport, ModelJson, RepairHeuristics, SessionCfg};
use crate::protocol_repair_prompt;
use crate::protocol_stream;
use crate::protocol_system_prompt;
use crate::protocol_wait::{self, ClarifyWaitOutcome};
use crate::{deepseek_api, git_utils, runner, workspace};
use crate::{protocol_failure, protocol_patch};
use crate::{protocol_history, protocol_session_state};
use std::sync::{atomic::AtomicBool, Arc};

// User-friendly defaults: keep these fixed (no UI inputs).
const FIXED_AUTO_REVERT_PROFILE: &str = "balanced";
const FIXED_EVAL_CMD: &str = "bash scripts/run_tests.sh";

/// Run the agent loop. This function listens for requests from the UI and
/// interacts with the DeepSeek API, applying patches and evaluating the
/// resulting project. It sends events back to the UI to update state.
pub async fn agent_loop(
    rx_req: Receiver<AgentRequest>,
    tx_evt: Sender<AgentEvent>,
    stop_now: Arc<AtomicBool>,
) {
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
                language,
                unattended_mode,
                resume_from_checkpoint,
                workspace,
                goal,
            }) => {
                stop_flag = false;
                let session_cfg = SessionCfg {
                    api_key,
                    base_url,
                    model,
                    language,
                    unattended_mode,
                    resume_from_checkpoint,
                    workspace,
                    goal,
                    eval_cmd: FIXED_EVAL_CMD.to_string(),
                };
                cfg = Some(session_cfg.clone());
                clarify_answers.clear();
                let _ = tx_evt.send(AgentEvent::Log("[Agent] Start received".into()));
                if let Err(e) = run_session(
                    &session_cfg,
                    &mut clarify_answers,
                    &rx_req,
                    &tx_evt,
                    &mut stop_flag,
                    &stop_now,
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
                    protocol_wait::handle_revert_request(&cfg.workspace, &tx_evt);
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
                    protocol_wait::handle_push_remote_request(
                        &cfg.workspace,
                        &tx_evt,
                        &remote,
                        &url,
                        &branch,
                    );
                }
            }
            Err(_) => break,
        }
    }

    // Read stop_flag to avoid unused assignment warnings. This has no
    // functional effect but ensures the compiler treats the variable as used.
    let _ = stop_flag;
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
    stop_now: &Arc<AtomicBool>,
) -> Result<()> {
    protocol_auto_revert::set_current_auto_revert_profile(FIXED_AUTO_REVERT_PROFILE);
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
        if let Some(state) = protocol_session_state::load_session_state(&cfg.workspace) {
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
        messages.push(deepseek_api::ChatMessage::system(
            protocol_system_prompt::system_prompt(&cfg.language, cfg.unattended_mode),
        ));
        let first_user = if cfg.unattended_mode {
            format!(
                "用户的需求如下：\n{}\n\n你需要作为自编程代理，根据该需求制定项目计划和目标，选择合适的技术栈并创建项目目录结构，编写代码，编译运行程序，分析并修复错误，如此往复循环，直至项目满足需求。为此，你应在项目目录中维护一个自动评测脚本（如 scripts/run_tests.sh），该脚本必须能够编译并运行程序、自动操作程序以执行必要的功能，并检测是否存在错误或未满足的目标。每次生成补丁后，你都需要更新这个评测脚本以反映新的需求。\n当前为无人值守模式：禁止输出 kind=clarify，必须直接基于合理默认假设输出 kind=patch JSON（diff 字段为 unified diff）并持续迭代。",
                cfg.goal
            )
        } else {
            format!(
                "用户的需求如下：\n{}\n\n你需要作为自编程代理，根据该需求制定项目计划和目标，选择合适的技术栈并创建项目目录结构，编写代码，编译运行程序，分析并修复错误，如此往复循环，直至项目满足需求。为此，你应在项目目录中维护一个自动评测脚本（如 scripts/run_tests.sh），该脚本必须能够编译并运行程序、自动操作程序以执行必要的功能，并检测是否存在错误或未满足的目标。每次生成补丁后，你都需要更新这个评测脚本以反映新的需求。\n首先，请输出 JSON（kind=clarify 或 kind=patch）：如需澄清问题，请用 kind=clarify，并提出关键问题；如无需澄清，请用 kind=patch，并给出 unified diff（含 hunk）的补丁。",
                cfg.goal
            )
        };
        messages.push(deepseek_api::ChatMessage::user(first_user));
    }

    // Save an initial checkpoint as soon as session context is ready.
    protocol_session_state::save_session_state(
        &cfg.workspace,
        &build_session_state(cfg, &messages, 0, "session_initialized"),
    );
    let mut repair = RepairHeuristics::default();
    'outer: for iter in 1..=30 {
        if *stop_flag || stop_now.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }
        if let Some((removed_msgs, removed_chars)) =
            protocol_history::trim_message_history_for_speed(&mut messages)
        {
            let _ = tx_evt.send(AgentEvent::Log(format!(
                "[Agent] 为提升性能已裁剪上下文: removed_messages={} removed_chars={}",
                removed_msgs, removed_chars
            )));
        }
        protocol_session_state::save_session_state(
            &cfg.workspace,
            &build_session_state(cfg, &messages, iter, "iteration_started"),
        );
        let _ = tx_evt.send(AgentEvent::Log(format!("\n[Loop] Iteration {}", iter)));
        let api_start = std::time::Instant::now();
        // Send the chat completion request to DeepSeek and keep emitting
        // progress logs so UI users can distinguish "still generating" from "stuck".
        let resp = match protocol_stream::chat_complete_with_progress(
            &cfg.base_url,
            &cfg.api_key,
            &cfg.model,
            &messages,
            tx_evt,
            stop_now,
        )
        .await
        {
            Ok(v) => v,
            Err(e) => {
                if protocol_stream::is_stop_requested_error(&e)
                    || stop_now.load(std::sync::atomic::Ordering::Relaxed)
                {
                    return Ok(());
                }
                return Err(e).context("deepseek chat_complete");
            }
        };
        // If the stop flag was set while waiting for the model, exit early.
        if stop_now.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }
        let api_ms = api_start.elapsed().as_millis();
        let _ = tx_evt.send(AgentEvent::Log(format!("[Perf] deepseek_api={}ms", api_ms)));
        let raw = resp.trim().to_string();
        if raw.len() > 200_000 {
            messages.push(deepseek_api::ChatMessage::assistant(
                protocol_history::compact_assistant_text(&raw, 12_000),
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
                    protocol_history::compact_assistant_text(&raw, 12_000),
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
                    protocol_history::compact_assistant_text(&raw, 12_000),
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
                if cfg.unattended_mode {
                    let count = questions.len();
                    let _ = tx_evt.send(AgentEvent::Log(format!(
                        "[Unattended] 模型返回 clarify（questions={}），已强制忽略并继续 patch 迭代",
                        count
                    )));
                    messages.push(deepseek_api::ChatMessage::assistant(content));
                    messages.push(deepseek_api::ChatMessage::user(
                        "无人值守模式禁止 kind=clarify。请基于现有上下文与合理默认假设直接输出 kind=patch JSON，并持续自我迭代直至满足目标。".to_string(),
                    ));
                    protocol_session_state::save_session_state(
                        &cfg.workspace,
                        &build_session_state(cfg, &messages, iter, "unattended_clarify_redirected"),
                    );
                    continue;
                }
                // Ask the UI for clarification answers.
                let _ = tx_evt.send(AgentEvent::NeedClarify { questions });
                match protocol_wait::wait_for_clarify_answers(
                    rx_req,
                    tx_evt,
                    &cfg.workspace,
                    stop_flag,
                ) {
                    ClarifyWaitOutcome::Answers(answers) => {
                        *clarify_answers = answers;
                    }
                    ClarifyWaitOutcome::Stopped => {
                        return Ok(());
                    }
                }
                // Send answers back to model and ask for a patch.
                let answers_json = serde_json::to_string_pretty(&to_answer_map(clarify_answers))?;
                messages.push(deepseek_api::ChatMessage::assistant(content));
                messages.push(deepseek_api::ChatMessage::user(format!(
                    "这是用户对澄清问题的回答(JSON)：\n{}\n\n现在请输出 kind=patch 的 JSON，给出最小可运行实现。要求：\n- 只输出 JSON（不要 markdown）\n- diff 必须是 unified diff（包含 `diff --git` 与 `@@` hunk）\n- 尽量小步提交，方便迭代。",
                    answers_json
                )));
                // Save session state after appending new messages so that we can resume later.
                protocol_session_state::save_session_state(
                    &cfg.workspace,
                    &build_session_state(cfg, &messages, iter, "waiting_patch_after_clarify"),
                );
            }
            ModelJson::Patch { summary, diff } => {
                // 记录 patch 输出方便之后放入对话历史
                let patch_content = content.clone();
                let diff = protocol_patch::sanitize_unified_diff(&diff);
                let patch_paths =
                    protocol_patch::extract_paths_from_unified_diff(&diff).unwrap_or_default();
                let patch_history =
                    protocol_history::compact_patch_history(&summary, &patch_paths, &patch_content);
                repair.total_patches += 1;
                if let Err(e) = validate_patch_payload(&summary, &diff) {
                    let _ =
                        tx_evt.send(AgentEvent::Log(format!("[Patch] 无效补丁，已拒绝：{}", e)));
                    messages.push(deepseek_api::ChatMessage::assistant(patch_content));
                    messages.push(deepseek_api::ChatMessage::user(format!(
                        "你给出的 patch 无效：{}。请重新输出 kind=patch JSON，并确保 diff 是可应用的 unified diff（含 `diff --git` 与 `@@` hunk），只做必要最小改动。",
                        e
                    )));
                    protocol_session_state::save_session_state(
                        &cfg.workspace,
                        &build_session_state(cfg, &messages, iter, "invalid_patch_payload"),
                    );
                    continue;
                }
                if let Err(e) = git_utils::check_apply_unified_diff(&cfg.workspace, &diff) {
                    let _ = tx_evt.send(AgentEvent::Log(format!(
                        "[Patch] diff 预检失败：{}",
                        truncate(&e.to_string(), 800)
                    )));
                    messages.push(deepseek_api::ChatMessage::assistant(patch_content));
                    messages.push(deepseek_api::ChatMessage::user(format!(
                        "你的 unified diff 无法应用：{}。{}",
                        truncate(&e.to_string(), 1200),
                        apply_failure_hint(&e.to_string())
                    )));
                    protocol_session_state::save_session_state(
                        &cfg.workspace,
                        &build_session_state(cfg, &messages, iter, "invalid_patch_apply_check"),
                    );
                    continue;
                }
                let _ = tx_evt.send(AgentEvent::Log(format!(
                    "[Patch] #{} {} | files={} diff_bytes={}",
                    repair.total_patches,
                    summary,
                    patch_paths.len(),
                    diff.len()
                )));
                if let Err(e) = git_utils::apply_unified_diff(&cfg.workspace, &diff) {
                    let _ = tx_evt.send(AgentEvent::Log(format!(
                        "[Patch] diff 应用失败：{}",
                        truncate(&e.to_string(), 800)
                    )));
                    messages.push(deepseek_api::ChatMessage::assistant(patch_content));
                    messages.push(deepseek_api::ChatMessage::user(format!(
                        "你的 unified diff 在应用阶段失败：{}。{}",
                        truncate(&e.to_string(), 1200),
                        apply_failure_hint(&e.to_string())
                    )));
                    protocol_session_state::save_session_state(
                        &cfg.workspace,
                        &build_session_state(cfg, &messages, iter, "invalid_patch_apply_exec"),
                    );
                    continue;
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
                // 评测流水线：固定执行 scripts/run_tests.sh（由模型维护）。
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
                    && !eval_has_fatal_runtime_marker(&result.stdout, &result.stderr);
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
                        && !eval_has_fatal_runtime_marker(&verify.stdout, &verify.stderr);
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
                        let repair_prompt = protocol_repair_prompt::build_repair_prompt(
                            &cfg.eval_cmd,
                            iter,
                            &verify,
                            &digest,
                            repair.consecutive_eval_failures,
                            repair.consecutive_same_failure,
                            &diff_for_feedback,
                        );
                        messages.push(deepseek_api::ChatMessage::user(format!(
                            "你上一次补丁在主评测通过，但在回归复测失败。请优先修复不稳定/漏测问题。\n{}",
                            repair_prompt
                        )));
                        protocol_session_state::save_session_state(
                            &cfg.workspace,
                            &build_session_state(cfg, &messages, iter, "eval_verify_failed"),
                        );
                        continue;
                    }
                    if cfg.unattended_mode {
                        protocol_session_state::save_session_state(
                            &cfg.workspace,
                            &build_session_state(cfg, &messages, iter, "done_unattended_success"),
                        );
                        let _ = tx_evt.send(AgentEvent::Done {
                            success: true,
                            message: "无人值守模式评测通过，自动结束".into(),
                        });
                        return Ok(());
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
                    match protocol_wait::wait_for_clarify_answers(
                        rx_req,
                        tx_evt,
                        &cfg.workspace,
                        stop_flag,
                    ) {
                        ClarifyWaitOutcome::Answers(answers) => {
                            let feedback = extract_feedback_text(&answers);
                            let trimmed = feedback.trim();
                            // 如果用户表示满意（空或含满意字样），结束会话；否则作为改进建议继续迭代。
                            if trimmed.is_empty() || trimmed.contains("满意") {
                                // 保存当前会话状态并结束。将来如果重新启动，用户需重新设定目标。
                                protocol_session_state::save_session_state(
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
                                protocol_session_state::save_session_state(
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
                        ClarifyWaitOutcome::Stopped => {
                            return Ok(());
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
                    messages.push(deepseek_api::ChatMessage::user(
                        protocol_repair_prompt::build_repair_prompt(
                            &cfg.eval_cmd,
                            iter,
                            &result,
                            &digest,
                            repair.consecutive_eval_failures,
                            repair.consecutive_same_failure,
                            &diff_for_feedback,
                        ),
                    ));
                    if should_auto_revert(&repair, &digest, previous_severity, iter) {
                        protocol_wait::handle_revert_request(&cfg.workspace, tx_evt);
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
                    protocol_session_state::save_session_state(
                        &cfg.workspace,
                        &build_session_state(cfg, &messages, iter, "eval_failed_waiting_fix"),
                    );
                    continue;
                }
            }
        }
    }
    // 达到最大迭代次数后保存状态，以便可能的续跑。
    protocol_session_state::save_session_state(
        &cfg.workspace,
        &build_session_state(cfg, &messages, 30, "max_iterations_reached"),
    );
    let _ = tx_evt.send(AgentEvent::Done {
        success: false,
        message: "达到最大迭代次数仍未收敛".into(),
    });
    Ok(())
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

fn extract_feedback_text(answers: &[ClarifyAnswer]) -> String {
    for ans in answers {
        if ans.id == "feedback" {
            if !ans.text.is_empty() {
                return ans.text.clone();
            }
            if !ans.single.is_empty() {
                return ans.single.clone();
            }
            if !ans.multi.is_empty() {
                return ans.multi.join(", ");
            }
            return String::new();
        }
    }
    String::new()
}

fn run_eval_pipeline(cfg: &SessionCfg, tx_evt: &Sender<AgentEvent>) -> Result<EvalPipelineReport> {
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

fn failure_severity(category: &str) -> u8 {
    protocol_failure::failure_severity(category)
}

fn category_changed_worse(previous: &str, current: &str) -> bool {
    protocol_failure::category_changed_worse(previous, current)
}

fn should_auto_revert(
    repair: &RepairHeuristics,
    digest: &FailureDigest,
    previous_severity: u8,
    iter: u32,
) -> bool {
    if !protocol_auto_revert::auto_revert_on_repeat() {
        return false;
    }
    let cooldown = protocol_auto_revert::auto_revert_cooldown_iters();
    if repair.last_auto_revert_iter > 0 && iter < repair.last_auto_revert_iter + cooldown {
        return false;
    }
    let repeated_limit = protocol_auto_revert::auto_revert_repeat_count();
    let severity_now = failure_severity(digest.category);
    let severity_gate = severity_now >= protocol_auto_revert::auto_revert_min_severity();
    let repeated_gate = repair.consecutive_same_failure >= repeated_limit;
    let min_delta = protocol_auto_revert::auto_revert_min_worse_delta();
    let worsened_gate = protocol_auto_revert::auto_revert_on_worse()
        && severity_now >= previous_severity.saturating_add(min_delta);
    let signature_gate = protocol_auto_revert::auto_revert_signature_allowed(&digest.signature);
    (repeated_gate || worsened_gate) && severity_gate && signature_gate
}

fn validate_patch_payload(summary: &str, diff: &str) -> Result<()> {
    protocol_patch::validate_patch_payload(summary, diff)
}

type FailureDigest = protocol_failure::FailureDigest;

fn digest_eval_failure(exit_code: i32, stdout: &str, stderr: &str) -> FailureDigest {
    protocol_failure::digest_eval_failure(exit_code, stdout, stderr)
}

fn eval_has_fatal_runtime_marker(stdout: &str, stderr: &str) -> bool {
    protocol_failure::eval_has_fatal_runtime_marker(stdout, stderr)
}

fn apply_failure_hint(err: &str) -> &'static str {
    let t = err.to_lowercase();
    if t.contains("patch failed") || t.contains("does not apply") {
        return "hunk 与当前文件不匹配，请缩小改动范围并更新 hunk 上下文后重试。";
    }
    if t.contains("corrupt patch") || t.contains("malformed patch") {
        return "diff 格式损坏，请输出标准 unified diff（包含 diff --git / --- / +++ / @@）。";
    }
    if t.contains("no such file") {
        return "目标文件路径不正确，请核对 b/<path> 与仓库实际相对路径。";
    }
    "请输出更小、更精确、可直接应用的 unified diff。"
}

/// Truncate a string to a maximum length for log display. If the string is
/// longer than `max`, an ellipsis and truncation indicator are appended.
fn truncate(s: &str, max: usize) -> String {
    protocol_patch::truncate(s, max)
}

/// 尝试从模型回复中提取可解析的 JSON 字符串。
/// 如果输入本身已经是合法 JSON，则直接返回。
/// 否则查找首个 '{' 与最后一个 '}' 之间的子串尝试解析。
fn extract_json(input: &str) -> Result<String> {
    protocol_patch::extract_json(input)
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
        updated_at_unix: protocol_session_state::now_unix(),
        message_count: messages.len(),
        messages: messages.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::{category_changed_worse, failure_severity, validate_patch_payload};
    use crate::deepseek_api::ChatMessage;
    use crate::protocol_auto_revert::{
        auto_revert_min_severity, auto_revert_on_repeat, auto_revert_repeat_count,
        auto_revert_signature_allowed,
    };
    use crate::protocol_history::{compact_assistant_text, trim_message_history_for_speed};

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
    fn validate_patch_rejects_empty_diff() {
        let err = validate_patch_payload("summary", "").expect_err("must fail");
        assert!(err.to_string().contains("diff"));
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
