use crossbeam_channel::Receiver;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::lock_utils::lock_recover;
use crate::protocol::{AgentEvent, ClarifyQuestion};
use crate::web_ui::RuntimeStatus;
use crate::web_ui_analytics;

pub(crate) const MAX_EVENTS_PER_PULL: usize = 300;
pub(crate) const MAX_EVENTS_PER_STREAM_BATCH: usize = 120;
const MAX_EVENT_BUFFER: usize = 5000;
const MAX_EVENT_BUFFER_BYTES: usize = 8 * 1024 * 1024;

pub(crate) type EventBuffer = VecDeque<(usize, SerializableEvent)>;

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
    events: &EventBuffer,
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
    events: Arc<Mutex<EventBuffer>>,
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
                let mut runtime = lock_recover(&runtime, "runtime");
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
                        if let Some(ms) =
                            web_ui_analytics::parse_perf_ms(line, "[Perf] deepseek_api=")
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
            let mut runtime = lock_recover(&runtime, "runtime");
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
    events: &Arc<Mutex<EventBuffer>>,
    event_bytes: &Arc<Mutex<usize>>,
    next_event_id: &Arc<Mutex<usize>>,
    serial: SerializableEvent,
) {
    let mut evts = lock_recover(events, "events");
    let mut bytes = lock_recover(event_bytes, "event_bytes");
    let mut next_id = lock_recover(next_event_id, "next_event_id");

    push_event_with_limits_inner(
        &mut evts,
        &mut bytes,
        &mut next_id,
        serial,
        MAX_EVENT_BUFFER,
        MAX_EVENT_BUFFER_BYTES,
    );
}

fn push_event_with_limits_inner(
    events: &mut EventBuffer,
    bytes: &mut usize,
    next_id: &mut usize,
    serial: SerializableEvent,
    max_events: usize,
    max_bytes: usize,
) {
    let size = serializable_event_size(&serial);
    *bytes = bytes.saturating_add(size);
    events.push_back((*next_id, serial));

    while events.len() > max_events || *bytes > max_bytes {
        if let Some((_, removed)) = events.pop_front() {
            *bytes = bytes.saturating_sub(serializable_event_size(&removed));
        } else {
            break;
        }
    }

    *next_id = next_id.saturating_add(1);
}

fn serializable_event_size(evt: &SerializableEvent) -> usize {
    match evt {
        SerializableEvent::Log { line } => line.len(),
        SerializableEvent::Diff { diff } => diff.len(),
        SerializableEvent::Done { message, .. } => message.len(),
        SerializableEvent::NeedClarify { questions } => questions
            .iter()
            .map(|q| {
                q.id.len()
                    + q.question.len()
                    + q.qtype.len()
                    + q.options.iter().map(|x| x.len()).sum::<usize>()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_events_filters_and_limits() {
        let mut buf: EventBuffer = Default::default();
        buf.push_back((1, SerializableEvent::Log { line: "a".into() }));
        buf.push_back((2, SerializableEvent::Log { line: "b".into() }));
        buf.push_back((3, SerializableEvent::Log { line: "c".into() }));

        let out = pull_events(&buf, 2, 2);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, 2);
        assert_eq!(out[1].0, 3);
    }

    #[test]
    fn push_event_evicts_oldest_by_count() {
        let mut buf: EventBuffer = Default::default();
        let mut bytes = 0usize;
        let mut next_id = 0usize;

        for i in 0..5 {
            push_event_with_limits_inner(
                &mut buf,
                &mut bytes,
                &mut next_id,
                SerializableEvent::Log {
                    line: format!("l{i}"),
                },
                3,
                10_000,
            );
        }
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.front().map(|x| x.0), Some(2));
        assert_eq!(buf.back().map(|x| x.0), Some(4));
    }

    #[test]
    fn push_event_evicts_oldest_by_bytes() {
        let mut buf: EventBuffer = Default::default();
        let mut bytes = 0usize;
        let mut next_id = 0usize;

        // Each entry is ~100 bytes, so max_bytes=250 keeps at most 2.
        for _ in 0..4 {
            push_event_with_limits_inner(
                &mut buf,
                &mut bytes,
                &mut next_id,
                SerializableEvent::Log {
                    line: "x".repeat(100),
                },
                100,
                250,
            );
        }
        assert!(buf.len() <= 2);
        assert!(bytes <= 250);
    }
}
