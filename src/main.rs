mod deepseek_api;
mod git_utils;
mod protocol;
mod runner;
mod web_ui;
mod web_ui_analytics;
mod web_ui_languages;
mod workspace;

use crossbeam_channel::unbounded;
use protocol::{AgentEvent, AgentRequest};

fn main() -> std::io::Result<()> {
    let (tx_req, rx_req) = unbounded::<AgentRequest>();
    let (tx_evt, rx_evt) = unbounded::<AgentEvent>();

    // Spawn the agent loop on a dedicated Tokio runtime.
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async move {
            protocol::agent_loop(rx_req, tx_evt).await;
        });
    });

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async { web_ui::run_web_server(tx_req, rx_evt).await })
}
