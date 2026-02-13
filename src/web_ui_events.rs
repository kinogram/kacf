use crossbeam_channel::Receiver;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::protocol::{AgentEvent, ClarifyQuestion};
use crate::web_ui::RuntimeStatus;
use crate::web_ui_analytics;

pub(crate) const MAX_EVENTS_PER_PULL: usize = 300;
pub(crate) const MAX_EVENTS_PER_STREAM_BATCH: usize = 120;
const MAX_EVENT_BUFFER: usize = 5000;
const MAX_EVENT_BUFFER_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SerializableEvent {
    Log { line: String },
    NeedClarify { questions: Vec<ClarifyQuestion> },
    Diff { diff: String },
    Done { success: bool, message: String },
}

impl From<AgentEvent> for SerializableEvent {
    fn from(evt: AgentEvent) -> Self {
        match evt {
            AgentEvent::Log(line) => SerializableEvent::Log { line },
            AgentEvent::NeedClarify { questions } => SerializableEvent::NeedClarify { questions },
            AgentEvent::Diff { diff } => SerializableEvent::Diff { diff },
            AgentEvent::Done { success, message } => SerializableEvent::Done { success, message },
        }
    }
}

pub(crate) fn pull_events(
    events: &[(usize, SerializableEvent)],
    from_id: usize,
    limit: usize,
) -> Vec<(usize, SerializableEvent)> {
    events
        .iter()
        .filter(|(id, _)| *id >= from_id)
        .take(limit)
        .cloned()
        .collect()
}

pub(crate) fn spawn_event_collector(
    rx_evt: Receiver<AgentEvent>,
    runtime: Arc<Mutex<RuntimeStatus>>,
    events: Arc<Mutex<Vec<(usize, SerializableEvent)>>>,
    event_bytes: Arc<Mutex<usize>>,
    next_event_id: Arc<Mutex<usize>>,
) {
    std::thread::spawn(move || {
        for evt in rx_evt.iter() {
            if let AgentEvent::Log(line) = &evt {
                if should_print_terminal_log(line) {
                    println!("{}", line);
                }
            }
            {
                let mut runtime = runtime.lock().unwrap();
                runtime.total_events += 1;
                match &evt {
                    AgentEvent::Log(line) => {
                        runtime.total_logs += 1;
                        if line.contains("失败") || line.to_lowercase().contains("error") {
                            runtime.last_error = line.clone();
                        }
                        if let Some(cat) = web_ui_analytics::parse_eval_digest_category(line) {
                            web_ui_analytics::push_digest(
                                &mut runtime.digest_history,
                                now_unix(),
                                cat,
                                600,
                            );
                        }
                        if let Some(sig) = web_ui_analytics::parse_eval_digest_signature(line) {
                            web_ui_analytics::push_digest(
                                &mut runtime.root_cause_history,
                                now_unix(),
                                sig,
                                600,
                            );
                        }
                        if let Some(ms) = web_ui_analytics::parse_perf_ms(line, "[Perf] deepseek_api=")
                        {
                            web_ui_analytics::push_sample(&mut runtime.api_ms_samples, ms, 400);
                        }
                        if let Some(ms) = web_ui_analytics::parse_eval_ms(line) {
                            web_ui_analytics::push_sample(&mut runtime.eval_ms_samples, ms, 400);
                        }
                    }
                    AgentEvent::Done { success, message } => {
                        runtime.running = false;
                        web_ui_analytics::push_done_history(
                            &mut runtime.done_history,
                            now_unix(),
                            *success,
                            500,
                        );
                        if *success {
                            runtime.total_done_ok += 1;
                        } else {
                            runtime.total_done_fail += 1;
                            runtime.last_error = message.clone();
                        }
                    }
                    _ => {}
                }
            }
            let serial: SerializableEvent = evt.clone().into();
            push_event_with_limits(&events, &event_bytes, &next_event_id, serial);
        }

        // If event channel closes unexpectedly while UI still thinks it's running,
        // force a terminal event so the WebUI can converge to a stopped state.
        let should_emit_done = {
            let mut runtime = runtime.lock().unwrap();
            if runtime.running {
                runtime.running = false;
                if runtime.last_error.trim().is_empty() {
                    runtime.last_error = "agent event channel closed".to_string();
                }
                true
            } else {
                false
            }
        };
        if should_emit_done {
            push_event_with_limits(
                &events,
                &event_bytes,
                &next_event_id,
                SerializableEvent::Done {
                    success: false,
                    message: "Agent event channel closed".to_string(),
                },
            );
        }
    });
}

fn push_event_with_limits(
    events: &Arc<Mutex<Vec<(usize, SerializableEvent)>>>,
    event_bytes: &Arc<Mutex<usize>>,
    next_event_id: &Arc<Mutex<usize>>,
    serial: SerializableEvent,
) {
    let mut evts = events.lock().unwrap();
    let mut bytes = event_bytes.lock().unwrap();
    let mut next_id = next_event_id.lock().unwrap();
    *bytes += serializable_event_size(&serial);
    evts.push((*next_id, serial));
    while evts.len() > MAX_EVENT_BUFFER || *bytes > MAX_EVENT_BUFFER_BYTES {
        if let Some((_, removed)) = evts.first() {
            *bytes = bytes.saturating_sub(serializable_event_size(removed));
        }
        evts.remove(0);
    }
    *next_id += 1;
}

fn serializable_event_size(evt: &SerializableEvent) -> usize {
    match evt {
        SerializableEvent::Log { line } => line.len(),
        SerializableEvent::Diff { diff } => diff.len(),
        SerializableEvent::Done { message, .. } => message.len(),
        SerializableEvent::NeedClarify { questions } => questions
            .iter()
            .map(|q| {
                q.id.len() + q.question.len() + q.qtype.len() + q.options.iter().map(|x| x.len()).sum::<usize>()
            })
            .sum(),
    }
}

fn should_print_terminal_log(line: &str) -> bool {
    let raw = line.trim_start();
    // Avoid printing model thinking/body stream to terminal to prevent huge output freezes.
    !(raw.starts_with("[Model-Stream]")
        || raw.starts_with("[Model-Thought]")
        || raw.starts_with("[Model] 仍在生成中...")
        || raw.starts_with("[Model] still generating..."))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
