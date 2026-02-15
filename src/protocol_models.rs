use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::deepseek_api;
use crate::runner;

#[derive(Debug)]
pub enum AgentRequest {
    Start {
        api_key: String,
        base_url: String,
        model: String,
        language: String,
        unattended_mode: bool,
        resume_from_checkpoint: bool,
        workspace: PathBuf,
        goal: String,
    },
    Clarify {
        answers: Vec<ClarifyAnswer>,
    },
    RevertLast,
    Stop,
    PushRemote {
        remote: String,
        url: String,
        branch: String,
    },
}

#[derive(Debug, Clone)]
pub enum AgentEvent {
    Log(String),
    NeedClarify { questions: Vec<ClarifyQuestion> },
    Diff { diff: String },
    Done { success: bool, message: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClarifyQuestion {
    pub id: String,
    pub question: String,
    #[serde(rename = "type")]
    pub qtype: String,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ClarifyAnswer {
    pub id: String,
    pub qtype: String,
    pub single: String,
    pub multi: Vec<String>,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub(crate) enum ModelJson {
    #[serde(rename = "clarify")]
    Clarify { questions: Vec<ClarifyQuestion> },
    #[serde(rename = "patch")]
    Patch { summary: String, diff: String },
}

#[derive(Clone)]
pub(crate) struct SessionCfg {
    pub(crate) api_key: String,
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) language: String,
    pub(crate) unattended_mode: bool,
    pub(crate) resume_from_checkpoint: bool,
    pub(crate) workspace: PathBuf,
    pub(crate) goal: String,
    pub(crate) eval_cmd: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SessionState {
    #[serde(default = "default_schema_version")]
    pub(crate) schema_version: u32,
    pub(crate) goal: String,
    #[serde(default)]
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) eval_cmd: String,
    #[serde(default)]
    pub(crate) iteration: u32,
    #[serde(default)]
    pub(crate) last_status: String,
    #[serde(default)]
    pub(crate) updated_at_unix: u64,
    #[serde(default)]
    pub(crate) message_count: usize,
    #[serde(default)]
    pub(crate) messages: Vec<deepseek_api::ChatMessage>,
}

fn default_schema_version() -> u32 {
    2
}

#[derive(Default)]
pub(crate) struct RepairHeuristics {
    pub(crate) total_patches: u32,
    pub(crate) consecutive_eval_failures: u32,
    pub(crate) consecutive_same_failure: u32,
    pub(crate) last_failure_signature: String,
    pub(crate) last_failure_severity: u8,
    pub(crate) last_failure_category: String,
    pub(crate) last_auto_revert_iter: u32,
}

#[derive(Clone)]
pub(crate) struct EvalPipelineReport {
    pub(crate) result: runner::EvalResult,
    pub(crate) stage: String,
}
