mod auth;
mod deepseek_api;
mod git_utils;
mod lock_utils;
mod protocol;
mod protocol_auto_revert;
mod protocol_failure;
mod protocol_history;
mod protocol_models;
mod protocol_patch;
mod protocol_repair_prompt;
mod protocol_session_state;
mod protocol_stream;
mod protocol_system_prompt;
mod protocol_wait;
mod runner;
mod self_debug;
mod starter_bootstrap;
mod web_ui;
mod web_ui_analytics;
mod web_ui_authz;
mod web_ui_cache_logic;
mod web_ui_debug;
mod web_ui_events;
mod web_ui_languages;
mod web_ui_models;
mod web_ui_projects;
mod web_ui_runtime_env;
mod web_ui_runtime_metrics;
mod web_ui_session;
mod web_ui_slug;
mod web_ui_store;
mod web_ui_vm;
mod workspace;

use crossbeam_channel::unbounded;
use protocol::{AgentEvent, AgentRequest};
use std::sync::{atomic::AtomicBool, Arc};

fn main() -> std::io::Result<()> {
    let (tx_req, rx_req) = unbounded::<AgentRequest>();
    let (tx_evt, rx_evt) = unbounded::<AgentEvent>();
    let stop_now = Arc::new(AtomicBool::new(false));

    // Spawn the agent loop on a dedicated Tokio runtime.
    let stop_now_agent = stop_now.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async move {
            protocol::agent_loop(rx_req, tx_evt, stop_now_agent).await;
        });
    });

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async { web_ui::run_web_server(tx_req, rx_evt, stop_now).await })
}
