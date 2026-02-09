mod deepseek_api;
mod protocol;
mod runner;
mod workspace;
mod git_utils;
mod web_ui;

use crossbeam_channel::{unbounded, Receiver, Sender};
use eframe::egui;
use protocol::{AgentEvent, AgentRequest, ClarifyAnswer};
use std::path::PathBuf;
use std::env;

fn main() -> eframe::Result<()> {
    // Determine mode based on command line arguments and environment. If "--web" is
    // present or we detect no graphical display (no DISPLAY/WAYLAND env vars),
    // run the HTTP server instead of the native GUI. This allows usage in
    // headless environments (e.g. Termux) where a GUI is unavailable.
    let args: Vec<String> = env::args().collect();
    let use_web_cli = args.iter().any(|a| a == "--web");
    let headless_env = std::env::var_os("DISPLAY").is_none()
        && std::env::var_os("WAYLAND_DISPLAY").is_none()
        && std::env::var_os("WAYLAND_SOCKET").is_none();
    if use_web_cli || headless_env {
        // Set up channels for communicating with the agent loop
        let (tx_req, rx_req) = unbounded::<AgentRequest>();
        let (tx_evt, rx_evt) = unbounded::<AgentEvent>();

        // Spawn the agent loop on its own Tokio runtime in a background thread.
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            rt.block_on(async move {
                protocol::agent_loop(rx_req, tx_evt).await;
            });
        });

        // Run the Actix-web server on a separate Tokio runtime. If the
        // server exits, just return Ok. Any error will panic via unwrap.
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            web_ui::run_web_server(tx_req, rx_evt)
                .await
                .expect("failed to run web server");
        });
        return Ok(());
    }

    // Default mode: run the native GUI using eframe
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "AutoCoding (DeepSeek) - Rust GUI",
        native_options,
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
}

/// Top-level application state. This struct stores configuration fields that
/// control how the agent interacts with the DeepSeek API as well as runtime
/// state such as logs, current questions and answers, and diff results. The
/// `running` flag indicates whether the agent loop is currently active.
struct App {
    // Configurable fields
    api_key: String,
    model: String,
    base_url: String,

    workspace_dir: String,
    goal: String,
    eval_cmd: String,
    success_regex: String,

    // Runtime state
    running: bool,
    log: String,
    last_diff: Option<String>,

    /// When true, a patch has been proposed and is awaiting user decision.
    awaiting_patch: bool,

    /// Whether custom fonts have been configured on the egui context. We set
    /// this flag after calling `configure_fonts()` in `update()` to avoid
    /// reconfiguring fonts on every frame.
    fonts_configured: bool,

    // Communication channels to/from the agent thread
    tx_req: Sender<AgentRequest>,
    rx_evt: Receiver<AgentEvent>,

    // Questions requiring clarification and answers provided by the user
    pending_questions: Vec<protocol::ClarifyQuestion>,
    pending_answers: Vec<ClarifyAnswer>,
}

impl App {
    fn new() -> Self {
        let (tx_req, rx_req) = unbounded::<AgentRequest>();
        let (tx_evt, rx_evt) = unbounded::<AgentEvent>();

        // Spawn the agent loop in a background thread with its own Tokio runtime.
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            rt.block_on(async move {
                protocol::agent_loop(rx_req, tx_evt).await;
            });
        });

        Self {
            api_key: std::env::var("DEEPSEEK_API_KEY").unwrap_or_default(),
            model: "deepseek-chat".to_string(),
            base_url: "https://api.deepseek.com".to_string(),
            workspace_dir: "./workspace".to_string(),
            goal: "做一个最小示例：生成一个 Rust CLI 项目，运行 cargo test 成功。".to_string(),
            eval_cmd: "cargo test".to_string(),
            success_regex: "".to_string(),

            running: false,
            log: String::new(),
            last_diff: None,
            awaiting_patch: false,
            fonts_configured: false,

            tx_req,
            rx_evt,
            pending_questions: vec![],
            pending_answers: vec![],
        }
    }

    fn append_log(&mut self, s: &str) {
        self.log.push_str(s);
        if !s.ends_with('\n') {
            self.log.push('\n');
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Configure custom fonts that support Chinese characters on first update.
        // egui's default font set may not include glyphs for CJK languages, which
        // results in missing glyphs rendered as empty squares. We embed a
        // Noto Sans CJK font and insert it as the highest priority font for
        // proportional and monospace families. The font file is located in
        // `fonts/NotoSansCJK-Regular.ttc` and embedded using include_bytes!.
        if !self.fonts_configured {
            let mut fonts = egui::FontDefinitions::default();
            // Insert our CJK font data. The key "noto_cjk" is arbitrary but
            // must be referenced in the families below.
            fonts.font_data.insert(
                "noto_cjk".to_owned(),
                egui::FontData::from_static(include_bytes!("../fonts/NotoSansCJK-Regular.ttc")),
            );
            // Prepend the CJK font to the proportional and monospace font families
            // so that its glyphs are used for characters missing in the default fonts.
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "noto_cjk".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, "noto_cjk".to_owned());
            ctx.set_fonts(fonts);
            self.fonts_configured = true;
        }
        // Poll events from the agent and update UI state accordingly.
        while let Ok(evt) = self.rx_evt.try_recv() {
            match evt {
                AgentEvent::Log(line) => {
                    self.append_log(&line);
                }
                AgentEvent::NeedClarify { questions } => {
                    self.pending_questions = questions;
                    self.pending_answers = self
                        .pending_questions
                        .iter()
                        .map(|q| ClarifyAnswer::empty(&q.id, q.qtype.clone()))
                        .collect();
                    self.append_log("[UI] 模型提出疑问/选项，需要你选择。");
                }
                AgentEvent::Diff { diff } => {
                    // Store the last diff received from the agent. Set flag
                    // indicating a patch decision is needed. The UI displays
                    // this in a collapsible section for easy viewing.
                    self.last_diff = Some(diff);
                    self.awaiting_patch = true;
                }
                AgentEvent::Done { success, message } => {
                    self.running = false;
                    self.append_log(&format!("[DONE] success={} | {}", success, message));
                    // When a session ends, clear any pending patch decision
                    self.awaiting_patch = false;
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("AutoCoding (DeepSeek) - AI 自编程系统");
            ui.separator();

            ui.label("DeepSeek 配置");
            ui.horizontal(|ui| {
                ui.label("API Key:");
                ui.text_edit_singleline(&mut self.api_key);
            });
            ui.horizontal(|ui| {
                ui.label("Base URL:");
                ui.text_edit_singleline(&mut self.base_url);
            });
            ui.horizontal(|ui| {
                ui.label("Model:");
                // Use from_id_salt instead of deprecated from_id_source
                egui::ComboBox::from_id_salt("model")
                    .selected_text(&self.model)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.model, "deepseek-chat".to_string(), "deepseek-chat");
                        ui.selectable_value(&mut self.model, "deepseek-reasoner".to_string(), "deepseek-reasoner");
                    });
            });
            ui.separator();
            ui.label("任务配置");
            ui.horizontal(|ui| {
                ui.label("Workspace:");
                ui.text_edit_singleline(&mut self.workspace_dir);
            });
            ui.label("Goal(大需求):");
            ui.text_edit_multiline(&mut self.goal);
            ui.horizontal(|ui| {
                ui.label("评测命令(成功判定):");
                ui.text_edit_singleline(&mut self.eval_cmd);
            });
            ui.horizontal(|ui| {
                ui.label("额外成功正则(可空):");
                ui.text_edit_singleline(&mut self.success_regex);
            });
            ui.separator();
            // Control buttons: start/continue, stop, and revert last commit.
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.running, egui::Button::new("开始/继续"))
                    .clicked()
                {
                    self.running = true;
                    self.append_log("[UI] 启动 Agent Loop...");
                    let req = AgentRequest::Start {
                        api_key: self.api_key.clone(),
                        base_url: self.base_url.clone(),
                        model: self.model.clone(),
                        workspace: PathBuf::from(self.workspace_dir.clone()),
                        goal: self.goal.clone(),
                        eval_cmd: self.eval_cmd.clone(),
                        success_regex: self.success_regex.clone(),
                    };
                    let _ = self.tx_req.send(req);
                }
                if ui.add_enabled(self.running, egui::Button::new("停止")).clicked() {
                    self.running = false;
                    let _ = self.tx_req.send(AgentRequest::Stop);
                    self.append_log("[UI] 已请求停止。");
                }
                if ui
                    .add_enabled(self.running, egui::Button::new("回滚上一次提交"))
                    .clicked()
                {
                    let _ = self.tx_req.send(AgentRequest::RevertLast);
                    self.append_log("[UI] 请求回滚最后一次提交...");
                }
            });
            ui.separator();
            ui.label("日志 / 过程：");
            egui::ScrollArea::vertical()
                .max_height(240.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.log)
                            .desired_rows(12)
                            .font(egui::TextStyle::Monospace),
                    );
                });
            // Show the most recent diff (if any) in a collapsible area. This
            // allows the user to inspect the changes that occurred in the
            // last patch commit.
            if let Some(diff) = &self.last_diff {
                ui.separator();
                ui.collapsing("最近差异 (从上一提交生成)", |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut diff.clone())
                            .desired_rows(8)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY),
                    );
                });
            }

            // When a patch diff is awaiting user decision, show accept/reject buttons
            if self.awaiting_patch {
                ui.separator();
                ui.label("补丁预览：请确认是否应用此补丁？");
                ui.horizontal(|ui| {
                    if ui.button("应用补丁").clicked() {
                        let _ = self.tx_req.send(AgentRequest::ApplyPatch { accept: true });
                        self.append_log("[UI] 用户选择应用补丁");
                        self.awaiting_patch = false;
                    }
                    if ui.button("拒绝补丁").clicked() {
                        let _ = self.tx_req.send(AgentRequest::ApplyPatch { accept: false });
                        self.append_log("[UI] 用户拒绝补丁");
                        self.awaiting_patch = false;
                    }
                });
            }
        });
        // Clarification modal: if there are pending questions from the agent
        // we pop up a window for the user to answer them. When submitted,
        // the answers are sent back to the agent via the channel.
        if !self.pending_questions.is_empty() {
            egui::Window::new("需要你选择 / 澄清")
                .collapsible(false)
                .resizable(true)
                .show(ctx, |ui| {
                    ui.label("模型在继续写代码前需要你选择/澄清：");
                    ui.separator();
                    for (i, q) in self.pending_questions.iter().enumerate() {
                        ui.label(format!("{}: {}", q.id, q.question));
                        match q.qtype.as_str() {
                            "single" => {
                                let ans = &mut self.pending_answers[i];
                                for opt in &q.options {
                                    ui.radio_value(&mut ans.single, opt.clone(), opt);
                                }
                            }
                            "multi" => {
                                let ans = &mut self.pending_answers[i];
                                for opt in &q.options {
                                    let checked = ans.multi.contains(opt);
                                    let mut c = checked;
                                    if ui.checkbox(&mut c, opt).changed() {
                                        if c {
                                            ans.multi.push(opt.clone());
                                        } else {
                                            ans.multi.retain(|x| x != opt);
                                        }
                                    }
                                }
                            }
                            "text" => {
                                let ans = &mut self.pending_answers[i];
                                ui.text_edit_multiline(&mut ans.text);
                            }
                            _ => {
                                ui.label("未知问题类型");
                            }
                        }
                        ui.separator();
                    }
                    if ui.button("提交选择并继续").clicked() {
                        let answers = self.pending_answers.clone();
                        self.pending_questions.clear();
                        self.pending_answers.clear();
                        let _ = self.tx_req.send(AgentRequest::Clarify { answers });
                        self.append_log("[UI] 已提交澄清答案，继续 Agent Loop...");
                    }
                });
        }
        ctx.request_repaint(); // keep UI responsive
    }
}