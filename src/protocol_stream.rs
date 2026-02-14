use anyhow::Result;
use crossbeam_channel::Sender;
use std::cell::RefCell;
use std::time::Duration;

use crate::deepseek_api;
use crate::protocol::AgentEvent;
use crate::protocol_patch;
use std::sync::{Arc, atomic::AtomicBool};

#[derive(Debug)]
struct StopRequested;

impl std::fmt::Display for StopRequested {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "stop requested")
    }
}

impl std::error::Error for StopRequested {}

pub(crate) fn is_stop_requested_error(err: &anyhow::Error) -> bool {
    err.chain().any(|e| e.is::<StopRequested>())
}

#[derive(Default)]
struct StreamPreview {
    tag: &'static str,
    buf: String,
    chars: usize,
    events: usize,
}

impl StreamPreview {
    fn new(tag: &'static str) -> Self {
        Self {
            tag,
            ..Self::default()
        }
    }

    fn feed(&mut self, delta: &str) {
        self.chars += delta.chars().count();
        self.events += 1;
        self.buf.push_str(delta);
    }

    fn should_flush(&self, delta: &str) -> bool {
        self.buf.len() >= 120 || delta.contains('\n') || delta.contains('}') || delta.contains(']')
    }

    fn flush_to_log(&mut self, tx_evt: &Sender<AgentEvent>, max: usize) {
        if self.buf.is_empty() {
            return;
        }
        let out = std::mem::take(&mut self.buf);
        let _ = tx_evt.send(AgentEvent::Log(format!(
            "{} {}",
            self.tag,
            protocol_patch::truncate(&out, max)
        )));
    }
}

pub(crate) async fn chat_complete_with_progress(
    base_url: &str,
    api_key: &str,
    model: &str,
    messages: &[deepseek_api::ChatMessage],
    tx_evt: &Sender<AgentEvent>,
    stop_now: &Arc<AtomicBool>,
) -> Result<String> {
    let _ = tx_evt.send(AgentEvent::Log("[Model] 正在请求模型响应...".to_string()));
    let started_at = std::time::Instant::now();
    let preview = RefCell::new(StreamPreview::new("[Model-Stream]"));
    let reasoning_preview = RefCell::new(StreamPreview::new("[Model-Thought]"));
    let mut on_delta = |delta: &str| {
        if delta.is_empty() {
            return;
        }
        let mut st = preview.borrow_mut();
        st.feed(delta);
        if st.should_flush(delta) {
            st.flush_to_log(tx_evt, 600);
        }
    };
    let mut on_reasoning = |delta: &str| {
        if delta.is_empty() {
            return;
        }
        let mut st = reasoning_preview.borrow_mut();
        st.feed(delta);
        if st.should_flush(delta) {
            st.flush_to_log(tx_evt, 600);
        }
    };
    let req = deepseek_api::chat_complete_streaming_with_reasoning(
        base_url,
        api_key,
        model,
        messages,
        &mut on_delta,
        &mut on_reasoning,
    );
    tokio::pin!(req);
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut stop_poll = tokio::time::interval(Duration::from_millis(200));
    stop_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut first_tick = true;
    loop {
        tokio::select! {
            out = &mut req => {
                {
                    let mut st = preview.borrow_mut();
                    st.flush_to_log(tx_evt, 600);
                }
                {
                    let mut reasoning = reasoning_preview.borrow_mut();
                    reasoning.flush_to_log(tx_evt, 600);
                }
                let st = preview.borrow();
                let reasoning = reasoning_preview.borrow();
                let _ = tx_evt.send(AgentEvent::Log(format!(
                    "[Model] 流式完成: answer_chunks={} answer_chars={} thought_chunks={} thought_chars={}",
                    st.events, st.chars, reasoning.events, reasoning.chars
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
            _ = stop_poll.tick() => {
                if stop_now.load(std::sync::atomic::Ordering::Relaxed) {
                    {
                        let mut st = preview.borrow_mut();
                        st.flush_to_log(tx_evt, 600);
                    }
                    {
                        let mut reasoning = reasoning_preview.borrow_mut();
                        reasoning.flush_to_log(tx_evt, 600);
                    }
                    let _ = tx_evt.send(AgentEvent::Log("[Model] 已请求停止，正在取消模型请求...".to_string()));
                    return Err(anyhow::Error::new(StopRequested));
                }
            }
        }
    }
}
