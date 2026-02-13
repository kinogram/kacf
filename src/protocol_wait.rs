use crossbeam_channel::{Receiver, Sender};
use std::path::Path;

use crate::git_utils;
use crate::protocol::{AgentEvent, AgentRequest, ClarifyAnswer};

pub enum ClarifyWaitOutcome {
    Answers(Vec<ClarifyAnswer>),
    Stopped,
}

pub fn handle_revert_request(workspace: &Path, tx_evt: &Sender<AgentEvent>) {
    if let Err(e) = git_utils::revert_last_commit(workspace) {
        let _ = tx_evt.send(AgentEvent::Log(format!("[Agent] 回滚失败: {e}")));
    } else {
        let _ = tx_evt.send(AgentEvent::Log("[Agent] 已回滚上一次提交".into()));
    }
}

pub fn handle_push_remote_request(
    workspace: &Path,
    tx_evt: &Sender<AgentEvent>,
    remote: &str,
    url: &str,
    branch: &str,
) {
    let res: anyhow::Result<()> = (|| {
        git_utils::add_remote(workspace, remote, url)?;
        git_utils::push(workspace, remote, branch)?;
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
            let _ = tx_evt.send(AgentEvent::Log(format!("[Agent] 推送远程失败: {:#}", e)));
        }
    }
}

pub fn wait_for_clarify_answers(
    rx_req: &Receiver<AgentRequest>,
    tx_evt: &Sender<AgentEvent>,
    workspace: &Path,
    stop_flag: &mut bool,
) -> ClarifyWaitOutcome {
    loop {
        if *stop_flag {
            return ClarifyWaitOutcome::Stopped;
        }
        match rx_req.recv() {
            Ok(AgentRequest::Clarify { answers }) => {
                return ClarifyWaitOutcome::Answers(answers);
            }
            Ok(AgentRequest::RevertLast) => {
                handle_revert_request(workspace, tx_evt);
            }
            Ok(AgentRequest::PushRemote {
                remote,
                url,
                branch,
            }) => {
                handle_push_remote_request(workspace, tx_evt, &remote, &url, &branch);
            }
            Ok(AgentRequest::Stop) => {
                *stop_flag = true;
                return ClarifyWaitOutcome::Stopped;
            }
            Ok(AgentRequest::Start { .. }) => {}
            Err(_) => {
                *stop_flag = true;
                let _ = tx_evt.send(AgentEvent::Log("[Agent] 请求通道已关闭".into()));
                return ClarifyWaitOutcome::Stopped;
            }
        }
    }
}
