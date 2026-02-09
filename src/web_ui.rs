//! Minimal web UI mode for the AutoCoding agent.
//!
//! This module exposes HTTP endpoints to control the agent loop and to
//! retrieve events (logs, diffs, clarifications, completion). It is
//! intended to allow running the system on machines without a native
//! desktop environment (e.g. Termux) and interacting through a
//! browser. The server listens on 0.0.0.0:8080 by default.

use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use crossbeam_channel::{Receiver, Sender};

use crate::protocol::{AgentEvent, AgentRequest, ClarifyAnswer, ClarifyQuestion};

/// Index HTML page embedded at compile time. This static page provides a
/// simple browser-based interface for starting a session, viewing logs and
/// diffs, answering clarification questions, and controlling patch decisions
/// and rollback. The HTML file lives under `static/index.html` and is
/// included at compile time via `include_str!`.
const INDEX_HTML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/static/index.html"));

/// Shared state between HTTP handlers. Holds channels to the agent and
/// a buffer of events to deliver to clients. Events are assigned
/// monotonically increasing IDs so clients can poll only new events.
#[derive(Clone)]
pub struct AppState {
    pub tx_req: Sender<AgentRequest>,
    pub rx_evt: Receiver<AgentEvent>,
    pub events: Arc<Mutex<Vec<(usize, SerializableEvent)>>>,
    pub next_event_id: Arc<Mutex<usize>>, 
}

/// Serializable representation of AgentEvent for HTTP JSON responses.
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SerializableEvent {
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

/// Payload to start a session. Mirrors AgentRequest::Start.
#[derive(Debug, Deserialize)]
struct StartPayload {
    api_key: String,
    base_url: String,
    model: String,
    workspace: String,
    goal: String,
    eval_cmd: String,
    success_regex: String,
}

/// Payload for clarification answers. Maps question IDs to values.
#[derive(Debug, Deserialize)]
struct ClarifyPayload {
    answers: serde_json::Value,
}

/// Payload for pushing to a remote. Contains the remote name (e.g. "origin"),
/// the full remote URL (including token if necessary) and the branch to push.
#[derive(Debug, Deserialize)]
struct PushPayload {
    remote: String,
    url: String,
    branch: String,
}

/// HTTP handler: start a new session.
async fn start_session(data: web::Data<AppState>, body: web::Json<StartPayload>) -> impl Responder {
    let payload = body.into_inner();
    let req = AgentRequest::Start {
        api_key: payload.api_key,
        base_url: payload.base_url,
        model: payload.model,
        workspace: payload.workspace.into(),
        goal: payload.goal,
        eval_cmd: payload.eval_cmd,
        success_regex: payload.success_regex,
    };
    if let Err(e) = data.tx_req.send(req) {
        return HttpResponse::InternalServerError().body(format!("send start failed: {}", e));
    }
    HttpResponse::Ok().body("started")
}

/// HTTP handler: provide clarification answers.
async fn answer_clarify(data: web::Data<AppState>, body: web::Json<ClarifyPayload>) -> impl Responder {
    let mut answers_vec = Vec::new();
    if let Some(map) = body.answers.as_object() {
        for (id, value) in map {
            let (qtype, single, multi, text) = match value {
                serde_json::Value::String(s) => ("single", s.clone(), Vec::new(), String::new()),
                serde_json::Value::Array(arr) => {
                    let multi = arr.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>();
                    ("multi", String::new(), multi, String::new())
                }
                _ => ("text", String::new(), Vec::new(), value.to_string()),
            };
            answers_vec.push(ClarifyAnswer {
                id: id.to_string(),
                qtype: qtype.to_string(),
                single,
                multi,
                text,
            });
        }
    }
    if let Err(e) = data.tx_req.send(AgentRequest::Clarify { answers: answers_vec }) {
        return HttpResponse::InternalServerError().body(format!("send clarify failed: {}", e));
    }
    HttpResponse::Ok().body("clarify sent")
}

/// HTTP handler: push current branch to remote. Accepts JSON with remote, url, branch.
async fn push_remote(data: web::Data<AppState>, body: web::Json<PushPayload>) -> impl Responder {
    let payload = body.into_inner();
    let req = AgentRequest::PushRemote {
        remote: payload.remote,
        url: payload.url,
        branch: payload.branch,
    };
    if let Err(e) = data.tx_req.send(req) {
        return HttpResponse::InternalServerError().body(format!("send push failed: {}", e));
    }
    HttpResponse::Ok().body("push sent")
}


/// HTTP handler: revert last commit.
async fn revert_last(data: web::Data<AppState>) -> impl Responder {
    if let Err(e) = data.tx_req.send(AgentRequest::RevertLast) {
        return HttpResponse::InternalServerError().body(format!("send revert failed: {}", e));
    }
    HttpResponse::Ok().body("revert sent")
}

/// HTTP handler: stop session.
async fn stop_session(data: web::Data<AppState>) -> impl Responder {
    if let Err(e) = data.tx_req.send(AgentRequest::Stop) {
        return HttpResponse::InternalServerError().body(format!("send stop failed: {}", e));
    }
    HttpResponse::Ok().body("stop sent")
}

/// HTTP handler: get events. Clients should poll with `from` query param to get new events.
async fn get_events(data: web::Data<AppState>, query: web::Query<std::collections::HashMap<String, String>>) -> impl Responder {
    let from_id: usize = query.get("from").and_then(|v| v.parse().ok()).unwrap_or(0);
    let events = data.events.lock().unwrap();
    let list: Vec<_> = events.iter().filter(|(id, _)| *id >= from_id).cloned().collect();
    HttpResponse::Ok().json(list)
}

/// Serve the static index page for the root path. This handler returns
/// a simple HTML page that implements a small front-end using plain
/// JavaScript to interact with the REST API exposed by this server.
async fn index_page() -> impl Responder {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(INDEX_HTML)
}

/// Background worker to pull events from agent and store them with IDs.
fn spawn_event_collector(state: AppState) {
    std::thread::spawn(move || {
        for evt in state.rx_evt.iter() {
            // If the event is a log, also print it to stdout. This provides
            // immediate feedback in the terminal when running in web mode.
            match &evt {
                AgentEvent::Log(line) => {
                    println!("{}", line);
                }
                _ => {}
            }
            let serial: SerializableEvent = evt.clone().into();
            let mut evts = state.events.lock().unwrap();
            let mut next_id = state.next_event_id.lock().unwrap();
            evts.push((*next_id, serial));
            *next_id += 1;
        }
    });
}

/// Run the web server on 0.0.0.0:8080. Spawns a background thread to collect events.
pub async fn run_web_server(tx_req: Sender<AgentRequest>, rx_evt: Receiver<AgentEvent>) -> std::io::Result<()> {
    let state = AppState {
        tx_req,
        rx_evt,
        events: Arc::new(Mutex::new(Vec::new())),
        next_event_id: Arc::new(Mutex::new(0)),
    };
    // Spawn event collector thread
    spawn_event_collector(state.clone());

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(state.clone()))
            .route("/", web::get().to(index_page))
            .route("/start", web::post().to(start_session))
            .route("/clarify", web::post().to(answer_clarify))
            // Patch decision route removed: patches are applied automatically in this version
            .route("/revert", web::post().to(revert_last))
            .route("/stop", web::post().to(stop_session))
            .route("/push", web::post().to(push_remote))
            .route("/events", web::get().to(get_events))
    })
    .bind(("0.0.0.0", 8080))?
    .run()
    .await
}