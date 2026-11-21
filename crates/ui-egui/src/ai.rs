//! The AI surface: the suite's connections form and chat panel, plus this app's three deck actions,
//! their dialogs and the action strip.
//!
//! Everything the user configures is a [`local_ai::Connection`] (provider preset, base URL, model)
//! plus a key that lives in the OS keychain. An action turns the deck (or one slide) into a
//! [`Request`], the model streams an answer, and [`localslides_ai::actions::interpret`] turns it
//! into an [`Outcome`] that is applied through the engine's own commands inside **one undo group**
//! ([`Session::grouped`]) - so a failed apply rolls back and the `.pptx` is never written by the AI
//! directly.
//!
//! The connections form, the chat panel and the chat's strip are `craft_ai_ui` (the suite's AI
//! surface, shared with the other apps). What the app adds is what only the app knows: the system
//! prompt, the deck's outline as the chat's context, the three deck actions and how an answer is
//! applied.
//!
//! The AI is desktop-only: `local-ai` reads the OS keychain and speaks HTTP, which the web build
//! does not have.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use craft_ai_ui as ai_ui;
use egui::{Context, RichText, Ui};
use localslides_ai::local_ai::{self, Connection, Msg};
use localslides_ai::{AI_APP_ID, AI_APP_URL, Backend, LiveBackend, Outcome, PlannedSlide, Request};
use localslides_engine::Session;
use serde_json::json;

use crate::SlideApp;

/// What a worker thread reports back.
enum Event {
    Delta(String),
    Done(Result<String, String>),
}

/// Where an outcome is applied, in engine terms.
#[derive(Clone)]
pub(crate) enum Apply {
    /// New slides at the end of the deck, under the theme the user picked (if any).
    Deck { theme: Option<String> },
    /// One slide's title and body.
    Slide { index: usize },
    /// Corrected text, by the index the answer names.
    Fix,
}

/// The request in flight.
struct Running {
    request: Request,
    apply: Apply,
    rx: Receiver<Event>,
    started_ms: f64,
    partial: String,
    cancelled: Arc<AtomicBool>,
}

/// How much of the deck's outline goes as the chat's context.
const CONTEXT_LIMIT: usize = 8_000;

/// The whole request budget: the turns plus the outline.
const BUDGET_CHARS: usize = 20_000;

/// The most slides the outline names (a huge deck is cut, never refused).
const OUTLINE_SLIDES: usize = 40;

/// The chat request in flight: the worker's channel and the flag that drops a stopped answer.
struct ChatRunning {
    rx: Receiver<Event>,
    cancelled: Arc<AtomicBool>,
}

/// One of the action dialogs.
pub enum Dialog {
    Generate { topic: String, slides: u32, theme: Option<String>, error: Option<String> },
    Edit { index: usize, title: String, body: String, instruction: String, error: Option<String> },
    Fix { all: bool, error: Option<String> },
}

/// The last deck the user generated, so its dialog opens pre-filled (the suite's "repeat" gesture).
#[derive(Clone, Debug, PartialEq)]
pub struct LastDeck {
    pub topic: String,
    pub slides: u32,
    pub theme: Option<String>,
}

/// The AI state the window owns.
pub struct Ai {
    pub backend: Arc<dyn Backend>,
    /// The connection in use, from the suite's store (one, like the other apps of the suite).
    connection: Option<Connection>,
    /// Whether the connections form is open.
    pub settings_open: bool,
    connections: ai_ui::ConnectionsPanel,
    /// The chat: the suite's panel, with the deck's outline as its context.
    chat: ai_ui::ChatPanel,
    /// The chat's strip (spinner, seconds, the stream's tail and Stop).
    strip: ai_ui::StatusStrip,
    /// Whether the chat is open (it takes width from the deck).
    pub chat_open: bool,
    chat_running: Option<ChatRunning>,
    /// The prompts' language (the system's, read once).
    language: String,
    pub dialog: Option<Dialog>,
    fetching: Option<Receiver<Result<Vec<String>, String>>>,
    running: Option<Running>,
    pub error: Option<String>,
    /// The last generated deck (repeat).
    pub last: Option<LastDeck>,
    /// The engine's built-in theme names, for the identity preset (design.themes).
    pub themes: Vec<String>,
}

impl Default for Ai {
    fn default() -> Self {
        let app = local_ai::App { id: AI_APP_ID, url: AI_APP_URL };
        let mut error = None;
        // One connection per app, the suite's standard: the first of the store is the one in use.
        let connection = match Connection::load_all(&app) {
            Ok(connections) => connections.into_iter().next(),
            Err(e) => {
                error = Some(format!("the AI connections could not be read: {e}"));
                None
            }
        };
        let mut chat = ai_ui::ChatPanel::default();
        // The last 8 turns and one budget for everything that leaves: the outline plus the
        // conversation.
        chat.set_policy(ai_ui::WindowPolicy { turns: ai_ui::Turns::LastN(8), total_budget_chars: BUDGET_CHARS });
        Self {
            backend: Arc::new(LiveBackend::new(AI_APP_ID, AI_APP_URL)),
            connection,
            settings_open: false,
            connections: ai_ui::ConnectionsPanel::default(),
            chat,
            strip: ai_ui::StatusStrip::default(),
            chat_open: false,
            chat_running: None,
            language: sys_locale::get_locale().unwrap_or_else(|| "en".to_string()),
            dialog: None,
            fetching: None,
            running: None,
            error,
            last: None,
            themes: Vec::new(),
        }
    }
}

impl Ai {
    fn app() -> local_ai::App {
        local_ai::App { id: AI_APP_ID, url: AI_APP_URL }
    }

    /// The model in use, for the panel's header.
    pub(crate) fn model(&self) -> &str {
        self.connection.as_ref().map(|connection| connection.model.as_str()).unwrap_or_default()
    }

    /// Whether the panel has somewhere to send a question.
    pub(crate) fn ready(&self) -> bool {
        self.connection.as_ref().is_some_and(|connection| !connection.model.trim().is_empty())
    }

    /// Whether an AI request is in flight (the actions and the chat take turns).
    pub(crate) fn busy(&self) -> bool {
        self.running.is_some() || self.chat_running.is_some()
    }

    /// Starts one request on a worker thread. The prompts follow the system's language.
    pub(crate) fn start(&mut self, ctx: &Context, request: Request, apply: Apply) -> Result<(), String> {
        if self.busy() {
            return Err("Another AI request is still running. Stop it or wait for it.".into());
        }
        let Some(connection) = self.connection.clone() else {
            return Err("No AI connection yet. Open AI > Connections and add one.".into());
        };
        let messages = localslides_ai::actions::messages(&request, &self.language)?;
        // Remember the request so the dialog can come back pre-filled (repeat).
        if let (Request::GenerateDeck { topic, slides }, Apply::Deck { theme }) = (&request, &apply) {
            self.last = Some(LastDeck { topic: topic.clone(), slides: *slides, theme: theme.clone() });
        }
        let (rx, cancelled) = spawn_stream(self.backend.clone(), ctx, connection, messages)?;
        self.error = None;
        self.running = Some(Running { request, apply, rx, started_ms: crate::clock::now_ms(), partial: String::new(), cancelled });
        Ok(())
    }

    /// Stops showing the answer. The request itself runs to its end in the worker (the shared
    /// client has no abort), so the result is discarded here instead.
    pub(crate) fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            running.cancelled.store(true, Ordering::Relaxed);
        }
    }

    /// Sends what the chat's composer holds, through the same client the actions use.
    fn chat_send(&mut self, ctx: &Context) {
        if self.busy() {
            return;
        }
        let Some(lines) = self.chat.take_request() else { return };
        let Some(connection) = self.connection.clone() else {
            self.chat.finish(Err("No AI connection yet. Open AI > Connections and add one.".into()));
            return;
        };
        let messages: Vec<Msg> = lines
            .into_iter()
            .map(|line| match line.role {
                ai_ui::Role::System => Msg::System(line.text),
                ai_ui::Role::User => Msg::User(line.text),
                ai_ui::Role::Assistant => Msg::Assistant(line.text),
            })
            .collect();
        match spawn_stream(self.backend.clone(), ctx, connection, messages) {
            Ok((rx, cancelled)) => {
                self.strip.begin();
                self.chat_running = Some(ChatRunning { rx, cancelled });
            }
            Err(error) => self.chat.finish(Err(error)),
        }
    }

    /// Stops the chat's answer: what arrived stays as a turn, and nothing is applied.
    fn chat_stop(&mut self) {
        if let Some(running) = self.chat_running.take() {
            running.cancelled.store(true, Ordering::Relaxed);
        }
        self.strip.finish();
        self.chat.stopped();
    }

    /// What the chat's worker delivered since the last frame.
    fn tick_chat(&mut self) {
        let mut deltas = String::new();
        let mut finished = None;
        {
            let Some(running) = self.chat_running.as_ref() else { return };
            loop {
                match running.rx.try_recv() {
                    Ok(Event::Delta(delta)) => deltas.push_str(&delta),
                    Ok(Event::Done(result)) => {
                        finished = Some(result);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        finished = Some(Err("the AI worker stopped".into()));
                        break;
                    }
                }
            }
        }
        if !deltas.is_empty() {
            self.strip.push(&deltas);
            self.chat.push(&deltas);
        }
        let Some(result) = finished else { return };
        self.strip.finish();
        self.chat.finish(result);
        self.chat_running = None;
    }

    /// Inserts the last answer as a new slide at the end of the deck: the first line is the title,
    /// the rest are the body's bullets (one command, one undo step, nothing overwritten).
    fn insert_answer(&mut self, session: &mut Session) -> bool {
        let Some(answer) = self.chat.last_answer().map(str::trim).map(str::to_string) else { return false };
        if answer.is_empty() || session.active().is_none() {
            return false;
        }
        let at = deck_len(session);
        let (title, body) = split_answer(&answer);
        session.execute("slide.new", &json!({ "layout": "titleAndContent", "title": title, "body": body, "at": at })).is_ok()
    }

    /// Polls the workers and the model fetch; applies a finished outcome. Returns true when a
    /// command changed the deck (the window then shows the new slide).
    pub(crate) fn tick(&mut self, session: &mut Session) -> bool {
        self.tick_models();
        self.tick_chat();
        let Some(running) = self.running.as_mut() else { return false };
        let mut finished = None;
        loop {
            match running.rx.try_recv() {
                Ok(Event::Delta(delta)) => running.partial.push_str(&delta),
                Ok(Event::Done(result)) => {
                    finished = Some(result);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = Some(Err("the AI worker stopped".into()));
                    break;
                }
            }
        }
        let Some(result) = finished else { return false };
        let Some(running) = self.running.take() else { return false };
        if running.cancelled.load(Ordering::Relaxed) {
            return false;
        }
        let answer = match result {
            Ok(answer) => answer,
            Err(error) => {
                self.error = Some(error);
                return false;
            }
        };
        match localslides_ai::actions::interpret(&running.request, &answer) {
            Ok(outcome) => match apply(session, &running.request, running.apply.clone(), outcome) {
                Ok(()) => true,
                Err(error) => {
                    // The suite's gesture for a refused artifact action: the dialog comes back with
                    // the engine's reason and the user's parameters, ready to adjust.
                    self.reopen_after_refusal(&running.request, running.apply, error);
                    false
                }
            },
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }

    /// Reopens the dialog a refused action came from, with the engine's reason. The reason goes
    /// into the dialog, not the strip: a retry is one edit away.
    fn reopen_after_refusal(&mut self, request: &Request, apply: Apply, error: String) {
        self.dialog = match (request, apply) {
            (Request::GenerateDeck { topic, slides }, Apply::Deck { theme }) => {
                Some(Dialog::Generate { topic: topic.clone(), slides: *slides, theme, error: Some(error) })
            }
            (Request::EditSlide { title, body, instruction, .. }, Apply::Slide { index }) => {
                Some(Dialog::Edit { index, title: title.clone(), body: body.clone(), instruction: instruction.clone(), error: Some(error) })
            }
            _ => {
                self.error = Some(error);
                None
            }
        };
    }

    fn tick_models(&mut self) {
        let Some(rx) = &self.fetching else { return };
        match rx.try_recv() {
            Ok(Ok(models)) => {
                self.connections.models = models;
                self.fetching = None;
            }
            Ok(Err(error)) => {
                self.connections.failed(error);
                self.fetching = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.fetching = None,
        }
    }

    /// The connections form: the suite's dialog, with this app's store and keychain behind it.
    fn settings(&mut self, ctx: &Context) {
        if !self.settings_open {
            self.connections.open = false;
            return;
        }
        if !self.connections.open {
            self.connections.set_presets(presets());
            self.connections.open_with(self.connection.as_ref().map(draft_of));
        }
        let labels = form_labels();
        match self.connections.show(ctx, &labels) {
            ai_ui::ConnectionsIntent::None => {}
            ai_ui::ConnectionsIntent::Cancel => {
                self.connections.open = false;
                self.settings_open = false;
            }
            ai_ui::ConnectionsIntent::Save(draft, key) => self.save(draft, key),
            ai_ui::ConnectionsIntent::LoadModels(draft, key) => self.fetch_models(ctx, draft, key),
        }
    }

    /// Stores the connection (the key in the keychain) and keeps it in use.
    fn save(&mut self, draft: ai_ui::ConnectionDraft, key: String) {
        let app = Self::app();
        let connection = connection_of(&draft);
        if !key.is_empty()
            && let Err(error) = connection.set_key(&app, &key)
        {
            self.connections.failed(format!("the key could not be saved: {error}"));
            return;
        }
        match Connection::save_all(&app, std::slice::from_ref(&connection)) {
            Ok(()) => {
                self.connection = Some(connection);
                self.chat.clear();
                self.connections.saved();
                self.settings_open = false;
            }
            Err(error) => self.connections.failed(error),
        }
    }

    /// Asks the provider for its model list, on a worker thread (the key as typed, or the saved one).
    fn fetch_models(&mut self, ctx: &Context, draft: ai_ui::ConnectionDraft, key: String) {
        if self.fetching.is_some() {
            return;
        }
        let app = Self::app();
        let connection = connection_of(&draft);
        let key = if key.is_empty() { connection.key(&app).unwrap_or_default() } else { key };
        let (tx, rx) = mpsc::channel();
        let worker = ctx.clone();
        std::thread::spawn(move || {
            let result = local_ai::list_models_with_key(&app, &connection, &key);
            let _ = tx.send(result);
            worker.request_repaint();
        });
        self.fetching = Some(rx);
    }

    /// Opens the generate dialog. It comes back pre-filled with the last request (the suite's
    /// "repeat" gesture), and lists the engine's own themes for the identity preset.
    pub(crate) fn open_generate(&mut self, session: &mut Session) {
        self.themes = engine_themes(session);
        let (topic, slides, theme) =
            self.last.as_ref().map_or((String::new(), 6, None), |last| (last.topic.clone(), last.slides, last.theme.clone()));
        self.dialog = Some(Dialog::Generate { topic, slides, theme, error: None });
    }

    /// Opens the rewrite dialog on the current slide, reading its text first.
    pub(crate) fn open_edit(&mut self, session: &mut Session) {
        let Some(index) = current_slide(session) else {
            self.error = Some(tl!("Open a presentation first.").into());
            return;
        };
        match slide_parts(session, index) {
            Ok(parts) => {
                self.dialog =
                    Some(Dialog::Edit { index, title: parts.title_text(), body: parts.body_text(), instruction: String::new(), error: None })
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub(crate) fn open_fix(&mut self, session: &mut Session) {
        if current_slide(session).is_none() {
            self.error = Some(tl!("Open a presentation first.").into());
            return;
        }
        self.dialog = Some(Dialog::Fix { all: false, error: None });
    }
}

/// The slide the window is on, if a presentation is open.
fn current_slide(session: &Session) -> Option<usize> {
    let doc = session.active()?;
    doc.doc.slides.get(doc.selection.slide).map(|_| doc.selection.slide)
}

/// How many slides the deck has.
fn deck_len(session: &Session) -> usize {
    session.active().map_or(0, |doc| doc.doc.slides.len())
}

/// One slide's title and body placeholders: the shape ids `text.set` takes and the text they hold.
struct SlideParts {
    title: Option<(u64, String)>,
    body: Option<(u64, String)>,
}

impl SlideParts {
    fn title_text(&self) -> String {
        self.title.as_ref().map(|(_, text)| text.clone()).unwrap_or_default()
    }

    fn body_text(&self) -> String {
        self.body.as_ref().map(|(_, text)| text.clone()).unwrap_or_default()
    }
}

/// Reads slide `index` through the engine's own inspection (`slide.inspect`), so the AI works on
/// exactly what the user sees.
fn slide_parts(session: &mut Session, index: usize) -> Result<SlideParts, String> {
    let info = session.execute("slide.inspect", &json!({ "index": index })).map_err(|e| e.to_string())?;
    let mut parts = SlideParts { title: None, body: None };
    let title_fallback = info["title"].as_str().unwrap_or("").to_string();
    if let Some(shapes) = info["shapes"].as_array() {
        for shape in shapes {
            let Some(id) = shape["id"].as_u64() else { continue };
            let text = shape["text"].as_str().unwrap_or("").to_string();
            match shape["placeholder"].as_str() {
                Some("title" | "ctrTitle") if parts.title.is_none() => parts.title = Some((id, text)),
                Some("body" | "subTitle" | "obj") if parts.body.is_none() => parts.body = Some((id, text)),
                _ => {}
            }
        }
    }
    if parts.title.is_none() && !title_fallback.trim().is_empty() {
        // A layout whose title is not a placeholder still names the slide; nothing is written to it.
        parts.title = Some((0, title_fallback));
    }
    Ok(parts)
}

/// The engine's built-in theme names (`design.themes`), for the identity preset's picker.
fn engine_themes(session: &mut Session) -> Vec<String> {
    session
        .execute("design.themes", &json!({}))
        .ok()
        .and_then(|value| {
            value["themes"].as_array().map(|themes| themes.iter().filter_map(|theme| theme["name"].as_str().map(str::to_string)).collect())
        })
        .unwrap_or_default()
}

/// Applies one outcome through the engine's commands, inside one undo group.
fn apply(session: &mut Session, request: &Request, target: Apply, outcome: Outcome) -> Result<(), String> {
    match (target, outcome) {
        (Apply::Deck { theme }, Outcome::Deck { slides }) => apply_deck(session, request, slides, theme.as_deref()),
        (Apply::Slide { index }, Outcome::Slide { title, body }) => apply_slide(session, request, index, title, body),
        (Apply::Fix, Outcome::Corrected { slides }) => {
            let mut writes = Vec::with_capacity(slides.len());
            for slide in slides {
                let parts = slide_parts(session, slide.index)?;
                writes.extend(
                    writes_for(slide.index, &parts, Some(slide.title), Some(slide.body)).map_err(|e| format!("slide {}: {e}", slide.index + 1))?,
                );
            }
            run_writes(session, request, writes)
        }
        (_, _) => Err("the model answered a shape that does not fit this action".into()),
    }
}

/// Appends the planned slides at the end of the deck, in order, as one undo step. Each slide's role
/// picks its layout; the theme, when the user chose one, is the deck's identity.
fn apply_deck(session: &mut Session, request: &Request, slides: Vec<PlannedSlide>, theme: Option<&str>) -> Result<(), String> {
    let at = deck_len(session);
    session
        .grouped(request.undo_label(), |s| {
            if let Some(theme) = theme {
                s.execute("design.theme", &json!({ "name": theme }))?;
            }
            for (offset, slide) in slides.iter().enumerate() {
                let body = slide.bullets.join("\n");
                s.execute("slide.new", &json!({ "layout": slide.role.layout(), "title": slide.title, "body": body, "at": at + offset }))?;
            }
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    // The first generated slide in view (a selection move: not part of the undo step).
    let _ = session.execute("slide.go", &json!({ "index": at }));
    Ok(())
}

/// Writes one slide's new title and body.
fn apply_slide(session: &mut Session, request: &Request, index: usize, title: Option<String>, body: Option<String>) -> Result<(), String> {
    let parts = slide_parts(session, index).map_err(|e| format!("slide {}: {e}", index + 1))?;
    let writes = writes_for(index, &parts, title, body).map_err(|e| format!("slide {}: {e}", index + 1))?;
    run_writes(session, request, writes)
}

/// The `text.set` writes a part pair needs, refusing a part the slide has no placeholder for.
///
/// A title with a placeholder id of 0 is the layout's own text (nothing to write): it is skipped.
fn writes_for(index: usize, parts: &SlideParts, title: Option<String>, body: Option<String>) -> Result<Vec<(usize, u64, String)>, String> {
    let mut writes = Vec::new();
    if let Some(text) = title.filter(|text| !text.trim().is_empty()) {
        match &parts.title {
            Some((0, _)) => {}
            Some((id, _)) => writes.push((index, *id, text)),
            None => return Err("this slide has no title placeholder".into()),
        }
    }
    if let Some(text) = body.filter(|text| !text.trim().is_empty()) {
        match &parts.body {
            Some((id, _)) => writes.push((index, *id, text)),
            None => return Err("this slide has no body placeholder".into()),
        }
    }
    if writes.is_empty() {
        return Err("there was nothing to write into this slide".into());
    }
    Ok(writes)
}

/// Runs the writes as one undo step. `text.set` edits the **current** slide's shapes, so each write
/// is preceded by its slide's `slide.go` (a selection move, not an undo step of its own).
fn run_writes(session: &mut Session, request: &Request, writes: Vec<(usize, u64, String)>) -> Result<(), String> {
    session
        .grouped(request.undo_label(), |s| {
            for (index, id, text) in writes {
                s.execute("slide.go", &json!({ "index": index }))?;
                s.execute("text.set", &json!({ "id": id, "text": text }))?;
            }
            Ok(())
        })
        .map_err(|e| e.to_string())
}

/// The error banner and the in-flight status, drawn **before** the canvas (the canvas takes the
/// rest of the window, so a strip after it would be clipped).
pub(crate) fn status_strip(app: &mut SlideApp, ui: &mut Ui) {
    let mut dismiss = false;
    if let Some(error) = app.ai.error.clone() {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(ui.visuals().error_fg_color, error);
            if ui.button(tl!("Dismiss")).clicked() {
                dismiss = true;
            }
        });
    }
    if dismiss {
        app.ai.error = None;
    }
    let Some(running) = app.ai.running.as_ref() else { return };
    let label = running.request.label();
    let subject = running.request.subject();
    let elapsed = ((crate::clock::now_ms() - running.started_ms) / 1000.0).max(0.0) as u64;
    let tail: String = running.partial.chars().rev().take(80).collect::<String>().chars().rev().collect::<String>();
    let mut stop = false;
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(format!("{label}: {subject}"));
        ui.label(RichText::new(format!("{elapsed}s")).weak());
        if !tail.trim().is_empty() {
            ui.label(RichText::new(tail.replace('\n', " ")).weak());
        }
        if ui.button(tl!("Stop")).clicked() {
            stop = true;
        }
    });
    if stop {
        app.ai.stop();
    }
}

/// The connections window and the action dialogs, drawn last so they sit above the deck.
pub(crate) fn windows(app: &mut SlideApp, ctx: &Context) {
    app.ai.settings(ctx);
    dialog_window(app, ctx);
}

/// The chat beside the deck: the suite's panel over this app's conversation.
pub(crate) fn chat_panel(app: &mut SlideApp, ui: &mut Ui) {
    if !app.ai.chat_open {
        return;
    }
    let ctx = ui.ctx().clone();
    // The app's side of the contract: what the panel is, and what goes as context.
    app.ai.chat.set_system(SYSTEM_PROMPT);
    app.ai.chat.set_context(chat_context(&mut app.session));
    let model = app.ai.model().to_string();
    let ready = app.ai.ready();
    let labels = chat_labels(&model, ready);
    egui::Panel::right("ai-chat").resizable(true).default_size(360.0).max_size(560.0).show(ui, |ui| {
        let strip = app.ai.strip.show(ui, ai_ui::StripLabels { stop: tl!("Stop"), dismiss: tl!("Dismiss") });
        if strip == ai_ui::StripAction::Stop {
            app.ai.chat_stop();
        }
        ui.separator();
        match app.ai.chat.show(ui, &labels) {
            ai_ui::ChatAction::None => {}
            ai_ui::ChatAction::Send => app.ai.chat_send(&ctx),
            ai_ui::ChatAction::Retry => {
                if app.ai.chat.retry() {
                    app.ai.chat_send(&ctx);
                }
            }
            ai_ui::ChatAction::Stop => app.ai.chat_stop(),
            ai_ui::ChatAction::Clear => app.ai.chat.clear(),
            ai_ui::ChatAction::Insert => {
                app.ai.insert_answer(&mut app.session);
            }
            ai_ui::ChatAction::Settings => app.ai.settings_open = true,
            ai_ui::ChatAction::Close => {
                app.ai.chat_stop();
                app.ai.chat_open = false;
            }
        }
    });
}

/// The system prompt: the app's vocabulary and what an answer may not claim (model-facing text).
const SYSTEM_PROMPT: &str = "You are an assistant inside a presentation app. The deck's outline \
     travels as context. Answer in the language of the question, plainly and briefly, the way a \
     slide reads. You cannot change the deck: when the user wants new slides, say what they should \
     say and let them use the app's own AI actions. Never claim you changed a slide.";

/// The deck's outline as the panel's context: one line per slide, within budget.
fn chat_context(session: &mut Session) -> Option<ai_ui::ContextSource> {
    let count = deck_len(session);
    if count == 0 {
        return None;
    }
    let mut text = String::new();
    let mut named = 0usize;
    for index in 0..count.min(OUTLINE_SLIDES) {
        let parts = slide_parts(session, index).ok()?;
        let title = parts.title_text().trim().to_string();
        let body = parts.body_text();
        let first = body.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("");
        // A slide with no text says nothing about the deck: it is not part of the outline.
        if title.is_empty() && first.is_empty() {
            continue;
        }
        named += 1;
        text.push_str(&format!("{}. {title}", index + 1));
        if !first.is_empty() {
            text.push_str(&format!(" — {first}"));
        }
        text.push('\n');
    }
    if named == 0 {
        return None;
    }
    let label = crate::i18n::tr_fmt("the deck · {count} slides", &[("count", &count.to_string())]);
    Some(ai_ui::ContextSource::new(ai_ui::ContextKind::Document, label, text, CONTEXT_LIMIT, ai_ui::Side::Head))
}

/// The context chip: what the last request really sent.
fn chip(info: &ai_ui::ContextInfo) -> String {
    let mut text = crate::i18n::tr_fmt(
        "{label} · {sent} of {total} characters",
        &[("label", &info.label), ("sent", &info.sent.to_string()), ("total", &info.total.to_string())],
    );
    if info.truncated() {
        text.push_str(tl!(" (cut)"));
    }
    if info.turns > 0 {
        text.push_str(&crate::i18n::tr_fmt(" · {turns}/{kept} turns", &[("turns", &info.turns.to_string()), ("kept", &info.kept.to_string())]));
    }
    text
}

/// The chat labels, in the app's language (the app's interface is English).
fn chat_labels<'a>(model: &'a str, ready: bool) -> ai_ui::ChatLabels<'a> {
    ai_ui::ChatLabels {
        title: tl!("AI assistant"),
        not_configured: if ready { "" } else { tl!("Configure a connection first.") },
        configure: tl!("Settings"),
        settings: tl!("Settings"),
        clear: tl!("Clear"),
        close: tl!("Close"),
        send: tl!("Send"),
        stop: tl!("Stop"),
        insert: tl!("Insert as a slide"),
        retry: tl!("Retry"),
        dismiss: tl!("Dismiss"),
        empty: tl!("Ask about the deck, or about anything."),
        input_hint: if ready { tl!("Write to the assistant.") } else { tl!("Configure a connection first.") },
        chip: &chip,
        model,
        suggestions: &[],
        to_end: tl!("To the end"),
    }
}

/// Starts one streamed request on a worker thread: the pieces arrive as events on the channel.
fn spawn_stream(
    backend: Arc<dyn Backend>,
    ctx: &Context,
    connection: Connection,
    messages: Vec<Msg>,
) -> Result<(Receiver<Event>, Arc<AtomicBool>), String> {
    let (tx, rx) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let worker_ctx = ctx.clone();
    std::thread::Builder::new()
        .name("localslides-ai".into())
        .spawn(move || {
            let mut on_delta = |delta: &str| {
                if worker_cancelled.load(Ordering::Relaxed) {
                    return;
                }
                let _ = tx.send(Event::Delta(delta.to_string()));
                worker_ctx.request_repaint();
            };
            let result = backend.stream(&connection, &messages, &mut on_delta);
            let _ = tx.send(Event::Done(result));
            worker_ctx.request_repaint();
        })
        .map_err(|e| format!("the AI worker could not start: {e}"))?;
    Ok((rx, cancelled))
}

/// The answer as one slide: the first line is the title, the rest the body's bullets.
fn split_answer(answer: &str) -> (String, String) {
    let mut lines = answer.lines().map(str::trim).filter(|line| !line.is_empty());
    let title = lines.next().map(clean_line).unwrap_or_default();
    let body = lines.map(clean_line).filter(|line| !line.is_empty()).collect::<Vec<_>>().join("\n");
    (title, body)
}

/// A line without its Markdown list/heading marks (a slide body is text, not Markdown).
fn clean_line(line: &str) -> String {
    line.trim_start_matches(['#', '-', '*', ' ', '\t']).trim().to_string()
}

/// The provider presets the form offers, from the shared client's table.
fn presets() -> Vec<ai_ui::Preset> {
    local_ai::PROVIDERS
        .iter()
        .map(|provider| ai_ui::Preset { id: provider.id.to_string(), label: provider.label.to_string(), base_url: provider.base_url.to_string() })
        .collect()
}

/// The draft the form shows for the connection in use.
fn draft_of(connection: &Connection) -> ai_ui::ConnectionDraft {
    ai_ui::ConnectionDraft { provider: connection.provider.clone(), base_url: connection.base_url.clone(), model: connection.model.clone() }
}

/// The connection a draft describes (one connection per app, so the id is fixed).
fn connection_of(draft: &ai_ui::ConnectionDraft) -> Connection {
    let label = local_ai::provider(&draft.provider).map(|provider| provider.label).unwrap_or(draft.provider.as_str());
    Connection {
        id: AI_APP_ID.to_lowercase(),
        provider: draft.provider.trim().to_string(),
        label: label.to_string(),
        base_url: draft.base_url.trim().to_string(),
        model: draft.model.trim().to_string(),
    }
}

/// The connections form's words.
fn form_labels<'a>() -> ai_ui::ConnectionsLabels<'a> {
    ai_ui::ConnectionsLabels {
        title: tl!("AI connection"),
        provider: tl!("Provider"),
        base_url: tl!("Base URL"),
        model: tl!("Model"),
        api_key: tl!("API key"),
        list_models: tl!("Fetch models"),
        save: tl!("Save"),
        cancel: tl!("Cancel"),
        hint_url: tl!("Empty uses the provider's own address."),
        hint_key: tl!("The key lives in the OS keychain; the connection file never holds it."),
        close: tl!("Close"),
        needs_key: tl!("Paste the API key first: this provider asks for one."),
        needs_url: tl!("Choose a provider or type its address first."),
        needs_model: tl!("Choose a model first (Fetch models lists them)."),
    }
}

fn dialog_window(app: &mut SlideApp, ctx: &Context) {
    let Some(mut dialog) = app.ai.dialog.take() else { return };
    let mut run: Option<(Request, Apply)> = None;
    let mut close = false;
    let mut error: Option<String> = None;
    let title = match &dialog {
        Dialog::Generate { .. } => tl!("Generate presentation"),
        Dialog::Edit { .. } => tl!("Rewrite slide"),
        Dialog::Fix { .. } => tl!("Correct slides"),
    };
    egui::Window::new(title).collapsible(false).resizable(false).default_width(500.0).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(
        ctx,
        |ui| match &mut dialog {
            Dialog::Generate { topic, slides, theme, error: dialog_error } => {
                ui.label(tl!("Describe the presentation: a topic, an audience, or a whole text to organize."));
                ui.add(egui::TextEdit::multiline(topic).desired_rows(4).desired_width(f32::INFINITY));
                ui.horizontal(|ui| {
                    ui.label(tl!("Slides"));
                    ui.add(egui::DragValue::new(slides).range(1..=localslides_ai::actions::MAX_SLIDES));
                });
                // The identity preset: the engine's own themes, "None" first (opt-in).
                ui.horizontal(|ui| {
                    ui.label(tl!("Theme"));
                    if ui.selectable_label(theme.is_none(), tl!("None")).clicked() {
                        *theme = None;
                    }
                    for name in &app.ai.themes {
                        if ui.selectable_label(theme.as_deref() == Some(name.as_str()), name).clicked() {
                            *theme = Some(name.clone());
                        }
                    }
                });
                if let Some(message) = dialog_error.as_ref() {
                    ui.colored_label(ui.visuals().error_fg_color, message);
                }
                ui.horizontal(|ui| {
                    if ui.button(tl!("Generate")).clicked() {
                        run = Some((Request::GenerateDeck { topic: topic.clone(), slides: *slides }, Apply::Deck { theme: theme.clone() }));
                    }
                    if ui.button(tl!("Cancel")).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::Edit { index, title, body, instruction, error: dialog_error } => {
                ui.label(crate::i18n::tr_fmt("Slide {n}:", &[("n", &(*index + 1).to_string())]));
                ui.label(RichText::new(title.as_str()).monospace());
                ui.label(RichText::new(body.as_str()).monospace().weak().small());
                ui.separator();
                ui.label(tl!("Instruction (for example: shorter, translate, reorder the bullets)."));
                ui.add(egui::TextEdit::multiline(instruction).desired_rows(3).desired_width(f32::INFINITY));
                if let Some(message) = dialog_error.as_ref() {
                    ui.colored_label(ui.visuals().error_fg_color, message);
                }
                ui.horizontal(|ui| {
                    if ui.button(tl!("Rewrite")).clicked() {
                        run = Some((
                            Request::EditSlide { index: *index, title: title.clone(), body: body.clone(), instruction: instruction.clone() },
                            Apply::Slide { index: *index },
                        ));
                    }
                    if ui.button(tl!("Cancel")).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::Fix { all, error: dialog_error } => {
                ui.label(tl!("Correct the spelling, grammar and punctuation. The meaning, the length and the language stay."));
                ui.checkbox(all, tl!("All slides (otherwise only this one)"));
                if let Some(message) = dialog_error.as_ref() {
                    ui.colored_label(ui.visuals().error_fg_color, message);
                }
                ui.horizontal(|ui| {
                    if ui.button(tl!("Correct")).clicked() {
                        match collect_slides(&mut app.session, *all) {
                            Ok(slides) => run = Some((Request::FixSlides { slides }, Apply::Fix)),
                            Err(message) => error = Some(message),
                        }
                    }
                    if ui.button(tl!("Cancel")).clicked() {
                        close = true;
                    }
                });
            }
        },
    );
    if let Some((request, apply)) = run {
        match app.ai.start(ctx, request, apply) {
            Ok(()) => close = true,
            Err(message) => error = Some(message),
        }
    }
    if close {
        return;
    }
    set_dialog_error(&mut dialog, error);
    app.ai.dialog = Some(dialog);
}

/// The slides the fix action sends: the current one, or the whole deck (capped, so one request
/// stays a request).
fn collect_slides(session: &mut Session, all: bool) -> Result<Vec<localslides_ai::SlideText>, String> {
    const MAX_FIX_SLIDES: usize = 25;
    let Some(current) = current_slide(session) else {
        return Err("Open a presentation first.".into());
    };
    let indices: Vec<usize> = if all { (0..deck_len(session)).take(MAX_FIX_SLIDES).collect() } else { vec![current] };
    let mut out = Vec::with_capacity(indices.len());
    for index in indices {
        let parts = slide_parts(session, index)?;
        out.push(localslides_ai::SlideText { index, title: parts.title_text(), body: parts.body_text() });
    }
    out.retain(|slide| !slide.title.trim().is_empty() || !slide.body.trim().is_empty());
    if out.is_empty() {
        return Err("there is no text to correct in these slides.".into());
    }
    Ok(out)
}

fn set_dialog_error(dialog: &mut Dialog, error: Option<String>) {
    if let Some(message) = error {
        match dialog {
            Dialog::Generate { error: dialog_error, .. } | Dialog::Edit { error: dialog_error, .. } | Dialog::Fix { error: dialog_error, .. } => {
                *dialog_error = Some(message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use localslides_ai::Role;

    fn deck() -> Session {
        Session::with_new()
    }

    fn slide_count(session: &Session) -> usize {
        session.doc().unwrap().doc.slides.len()
    }

    fn undo_steps(session: &Session) -> usize {
        session.doc().unwrap().history.undo.len()
    }

    /// The text of slide `index`'s placeholder, as the engine holds it. `which` is `title` or
    /// `body`; a layout names them differently (`ctrTitle`/`subTitle` on a title slide).
    fn placeholder_text(session: &mut Session, index: usize, which: &str) -> Option<String> {
        let info = session.execute("slide.inspect", &json!({ "index": index })).unwrap();
        let shapes = info["shapes"].as_array()?;
        let kinds: &[&str] = if which == "title" { &["title", "ctrTitle"] } else { &["body", "subTitle", "obj"] };
        let shape = shapes.iter().find(|shape| shape["placeholder"].as_str().is_some_and(|name| kinds.contains(&name)))?;
        shape["text"].as_str().map(str::to_string)
    }

    #[test]
    fn a_planned_deck_is_appended_in_one_undo_step() {
        let mut session = deck();
        let before = slide_count(&session);
        let undo_before = undo_steps(&session);
        let request = Request::GenerateDeck { topic: "Luz".into(), slides: 2 };
        let outcome = Outcome::Deck {
            slides: vec![
                PlannedSlide { title: "Fotossíntese".into(), bullets: Vec::new(), role: Role::Title },
                PlannedSlide { title: "Luz".into(), bullets: vec!["Clorofila".into(), "Água".into()], role: Role::Content },
            ],
        };
        apply(&mut session, &request, Apply::Deck { theme: None }, outcome).expect("the deck is applied");
        assert_eq!(slide_count(&session), before + 2);
        assert_eq!(undo_steps(&session), undo_before + 1, "the whole deck is one undo step");
        let at = before;
        assert_eq!(placeholder_text(&mut session, at, "title").as_deref(), Some("Fotossíntese"));
        assert_eq!(placeholder_text(&mut session, at + 1, "title").as_deref(), Some("Luz"));
        assert_eq!(placeholder_text(&mut session, at + 1, "body").as_deref(), Some("Clorofila\nÁgua"));
        session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(slide_count(&session), before, "one undo takes the whole deck back");
    }

    /// Each role the model labels lands on the engine's own layout for it.
    #[test]
    fn each_role_lands_on_its_layout() {
        let mut session = deck();
        let request = Request::GenerateDeck { topic: "Luz".into(), slides: 3 };
        let outcome = Outcome::Deck {
            slides: vec![
                PlannedSlide { title: "Deck".into(), bullets: Vec::new(), role: Role::Title },
                PlannedSlide { title: "Parte".into(), bullets: Vec::new(), role: Role::Section },
                PlannedSlide { title: "Um".into(), bullets: vec!["x".into()], role: Role::Content },
            ],
        };
        apply(&mut session, &request, Apply::Deck { theme: None }, outcome).expect("the deck is applied");
        let layout = |session: &mut Session, index: usize| {
            session.execute("slide.inspect", &json!({ "index": index })).unwrap()["layout"].as_str().unwrap_or_default().to_string()
        };
        assert_eq!(layout(&mut session, 1), "Title Slide");
        assert_eq!(layout(&mut session, 2), "Section Header");
        assert_eq!(layout(&mut session, 3), "Title and Content");
    }

    /// A bullet with a leading tab becomes a second-level bullet: the placeholder carries levels.
    #[test]
    fn a_nested_bullet_lands_on_the_next_level() {
        let mut session = deck();
        let request = Request::GenerateDeck { topic: "Luz".into(), slides: 1 };
        let outcome =
            Outcome::Deck { slides: vec![PlannedSlide { title: "Um".into(), bullets: vec!["topo".into(), "\tfilho".into()], role: Role::Content }] };
        apply(&mut session, &request, Apply::Deck { theme: None }, outcome).expect("the deck is applied");
        let info = session.execute("slide.inspect", &json!({ "index": 1 })).unwrap();
        let body = info["shapes"]
            .as_array()
            .and_then(|shapes| {
                shapes.iter().find(|shape| shape["placeholder"].as_str() == Some("obj") || shape["placeholder"].as_str() == Some("body"))
            })
            .and_then(|shape| shape["id"].as_u64())
            .expect("the content placeholder");
        let got = session.execute("text.get", &json!({ "id": body })).unwrap();
        assert_eq!(got["paragraphs"][1]["level"], json!(1), "the tab is the level: {got}");
        assert_eq!(got["paragraphs"][1]["text"], "filho");
    }

    /// The theme the user picked is the deck's identity, applied in the same undo step.
    #[test]
    fn a_theme_preset_applies_to_the_deck() {
        let mut session = deck();
        let request = Request::GenerateDeck { topic: "Luz".into(), slides: 1 };
        let outcome = Outcome::Deck { slides: vec![PlannedSlide { title: "Um".into(), bullets: Vec::new(), role: Role::Content }] };
        apply(&mut session, &request, Apply::Deck { theme: Some("Harbor".into()) }, outcome).expect("the deck is applied");
        let name = session.execute("design.themes", &json!({})).ok().map(|value| value["themes"][0]["name"].clone());
        assert!(name.is_some(), "the engine lists its themes");
        let master = session.execute("slide.inspect", &json!({ "index": 0 })).unwrap();
        assert!(master["layout"].is_string());
        // The engine's own identity check: the deck's first master carries the theme by name.
        let doc = session.active().expect("a document");
        assert_eq!(doc.doc.masters[0].theme.name, "Harbor", "the chosen theme is the deck's");
    }

    #[test]
    fn a_rewritten_slide_writes_the_title_and_the_body() {
        let mut session = deck();
        let undo_before = undo_steps(&session);
        let request = Request::EditSlide { index: 0, title: "Antigo".into(), body: "velho".into(), instruction: "encurta".into() };
        let outcome = Outcome::Slide { title: Some("Novo".into()), body: Some("curto".into()) };
        apply(&mut session, &request, Apply::Slide { index: 0 }, outcome).expect("the slide is rewritten");
        assert_eq!(placeholder_text(&mut session, 0, "title").as_deref(), Some("Novo"));
        assert_eq!(placeholder_text(&mut session, 0, "body").as_deref(), Some("curto"));
        assert_eq!(undo_steps(&session), undo_before + 1);
    }

    #[test]
    fn a_slide_without_a_body_placeholder_refuses_the_body() {
        let mut session = deck();
        session.execute("slide.new", &json!({ "layout": "blank" })).unwrap();
        let request = Request::EditSlide { index: 1, title: String::new(), body: String::new(), instruction: "x".into() };
        let error = apply(&mut session, &request, Apply::Slide { index: 1 }, Outcome::Slide { title: None, body: Some("texto".into()) })
            .expect_err("no body placeholder");
        assert!(error.contains("body placeholder"), "{error}");
    }

    #[test]
    fn corrections_write_each_slide_inside_one_undo_step() {
        let mut session = deck();
        session.execute("slide.new", &json!({ "title": "Dois", "body": "texto" })).unwrap();
        let undo_before = undo_steps(&session);
        let request = Request::FixSlides { slides: vec![localslides_ai::SlideText { index: 0, title: "Intrudução".into(), body: String::new() }] };
        let outcome = Outcome::Corrected {
            slides: vec![
                localslides_ai::SlideText { index: 0, title: "Introdução".into(), body: String::new() },
                localslides_ai::SlideText { index: 1, title: "Dois".into(), body: "Texto.".into() },
            ],
        };
        apply(&mut session, &request, Apply::Fix, outcome).expect("the corrections are applied");
        assert_eq!(placeholder_text(&mut session, 0, "title").as_deref(), Some("Introdução"));
        assert_eq!(placeholder_text(&mut session, 1, "body").as_deref(), Some("Texto."));
        assert_eq!(undo_steps(&session), undo_before + 1, "one undo step for the whole correction");
    }

    // ---- the suite's chat surface ------------------------------------------------------------

    fn ai() -> Ai {
        let mut chat = ai_ui::ChatPanel::default();
        chat.set_policy(ai_ui::WindowPolicy { turns: ai_ui::Turns::LastN(8), total_budget_chars: BUDGET_CHARS });
        Ai {
            backend: Arc::new(LiveBackend::new(AI_APP_ID, AI_APP_URL)),
            connection: None,
            settings_open: false,
            connections: ai_ui::ConnectionsPanel::default(),
            chat,
            strip: ai_ui::StatusStrip::default(),
            chat_open: false,
            chat_running: None,
            language: "en".into(),
            dialog: None,
            fetching: None,
            running: None,
            error: None,
            last: None,
            themes: Vec::new(),
        }
    }

    /// A backend that answers from a canned string, one piece at a time: what the UI path uses.
    struct Canned(&'static str);

    impl Backend for Canned {
        fn stream(&self, _: &Connection, _: &[Msg], on_delta: &mut dyn FnMut(&str)) -> Result<String, String> {
            on_delta(self.0);
            Ok(self.0.to_string())
        }

        fn models(&self, _: &Connection) -> Result<Vec<String>, String> {
            Ok(Vec::new())
        }
    }

    /// The policy the panel asks for: the last 8 turns and one budget for everything that leaves.
    #[test]
    fn the_chat_keeps_the_last_turns_within_the_budget() {
        let ai = ai();
        let policy = ai.chat.policy();
        assert_eq!(policy.turns, ai_ui::Turns::LastN(8));
        assert_eq!(policy.total_budget_chars, BUDGET_CHARS);
    }

    /// The deck's outline is what travels: one line per slide, the title and its first bullet.
    #[test]
    fn the_deck_outline_is_the_chats_context() {
        let mut session = deck();
        session.execute("slide.new", &json!({ "title": "Dois", "body": "Clorofila\nÁgua" })).unwrap();
        let context = chat_context(&mut session).expect("the deck's outline");
        assert_eq!(context.kind, ai_ui::ContextKind::Document);
        assert_eq!(context.side, ai_ui::Side::Head);
        assert!(context.label.contains("2 slides"), "{}", context.label);
        assert!(context.text.contains("2. Dois — Clorofila"), "{}", context.text);
        assert!(!context.text.contains("1. "), "the empty title slide is not named: {}", context.text);
    }

    /// An empty deck sends no context at all.
    #[test]
    fn an_empty_deck_sends_no_context() {
        let mut session = Session::with_new();
        assert!(chat_context(&mut session).is_none());
    }

    /// Insert turns the answer into a new slide at the end: the first line is the title, the rest is
    /// the body, and nothing of the deck is overwritten.
    #[test]
    fn the_answer_is_inserted_as_a_new_slide() {
        let mut session = deck();
        let before = slide_count(&session);
        let mut ai = ai();
        ai.chat.push_answer("Energia\n- Fotossíntese\n- Respiração");
        assert!(ai.insert_answer(&mut session));
        assert_eq!(slide_count(&session), before + 1);
        let at = before;
        assert_eq!(placeholder_text(&mut session, at, "title").as_deref(), Some("Energia"));
        assert_eq!(placeholder_text(&mut session, at, "body").as_deref(), Some("Fotossíntese\nRespiração"));
    }

    /// With no deck open and no answer there is nothing to insert.
    #[test]
    fn an_empty_chat_inserts_nothing() {
        let mut session = deck();
        let before = slide_count(&session);
        let mut ai = ai();
        assert!(!ai.insert_answer(&mut session));
        assert_eq!(slide_count(&session), before);
    }

    /// A question typed in the panel goes through the app's own client and the answer lands in the
    /// panel: the shared panel owns the conversation, the app owns the transport.
    #[test]
    fn a_chat_question_goes_through_the_apps_client() {
        let ctx = Context::default();
        let mut session = deck();
        let mut ai = ai();
        ai.connection =
            Some(Connection { id: "test".into(), provider: "custom".into(), label: "Test".into(), base_url: String::new(), model: "mock".into() });
        ai.backend = Arc::new(Canned("quarenta e dois"));
        assert!(ai.chat.ask("quanto é?"), "the panel took the question");
        ai.chat_send(&ctx);
        assert!(ai.chat_running.is_some(), "the worker was started");
        for _ in 0..400 {
            ai.tick(&mut session);
            if ai.chat_running.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(ai.chat_running.is_none(), "the answer finished");
        assert_eq!(ai.chat.last_answer(), Some("quarenta e dois"));
        assert!(ai.strip.error().is_none(), "no error in the strip");
    }

    /// The key the ignored real-model test uses: the environment first, then the suite's
    /// `ai-test.env` on the Desktop (never printed).
    fn real_key() -> String {
        if let Ok(key) = std::env::var("OPENROUTER_API_KEY")
            && !key.trim().is_empty()
        {
            return key.trim().to_string();
        }
        let path = std::path::Path::new(r"C:\Users\Hades\Desktop\ai-test.env");
        let text = std::fs::read_to_string(path).expect("ai-test.env on the Desktop");
        text.lines()
            .filter_map(|line| line.split_once('='))
            .find(|(name, _)| name.trim() == "OPENROUTER_API_KEY")
            .map(|(_, value)| value.trim().trim_matches('"').to_string())
            .expect("OPENROUTER_API_KEY in ai-test.env")
    }

    /// The real-model chat, end to end: network and a key, so `#[ignore]`. Run with
    /// `AI_TEST_MODEL=nvidia/nemotron-3-ultra-550b-a55b:free cargo test -p localslides-ui-egui --lib ai::tests::a_real_model_answers_the_chat -- --ignored --nocapture`.
    #[test]
    #[ignore = "network + a real provider key"]
    fn a_real_model_answers_the_chat() {
        let key = real_key();
        let connection = Connection {
            id: "real".into(),
            provider: "openrouter".into(),
            label: "OpenRouter".into(),
            base_url: String::new(),
            model: std::env::var("AI_TEST_MODEL").unwrap_or_else(|_| "nvidia/nemotron-3-ultra-550b-a55b:free".into()),
        };
        let mut session = deck();
        session.execute("slide.new", &json!({ "title": "Fotossíntese", "body": "Clorofila converte luz" })).unwrap();
        let context = chat_context(&mut session).expect("the outline");
        let messages = vec![
            Msg::System(SYSTEM_PROMPT.to_string()),
            Msg::System(format!("{}\n\n{}", context.label, context.text)),
            Msg::User("Qual é o título da segunda slide?".into()),
        ];
        let mut pieces = 0;
        let answer =
            local_ai::chat_stream_with_key(&local_ai::App { id: AI_APP_ID, url: AI_APP_URL }, &connection, &key, &messages, &mut |_: &str| {
                pieces += 1
            })
            .expect("a real answer");
        println!("--- answer ({pieces} pieces) ---\n{answer}\n--------------");
        assert!(answer.to_lowercase().contains("fotoss"), "{answer}");
        assert!(pieces > 1, "the answer streamed in pieces");
    }

    /// Visual QA (ignored in CI): renders the assistant panel and the connections form in both
    /// themes to PNGs for the vision pass. Run with
    /// `cargo test -p localslides-ui-egui --lib ai::tests::renders_the_ai_surfaces_for_visual_qa -- --ignored --nocapture`.
    #[test]
    #[ignore = "visual QA: renders the AI surfaces to PNGs for review"]
    fn renders_the_ai_surfaces_for_visual_qa() {
        use crate::{Services, SlideApp};
        fn app(brightness: crate::theme::Brightness) -> SlideApp {
            let mut session = deck();
            session.execute("slide.new", &json!({ "title": "Luz", "body": "Clorofila\nÁgua" })).unwrap();
            let mut app = SlideApp::new(session, Services::default());
            app.ui.brightness = brightness;
            app.ai.connection = Some(Connection {
                id: "qa".into(),
                provider: "custom".into(),
                label: "Local QA".into(),
                base_url: String::new(),
                model: "qa-model".into(),
            });
            app
        }
        fn draw(ui: &mut Ui, app: &mut SlideApp) {
            app.logic(ui.ctx());
            app.ui(ui);
        }
        fn save(h: &mut egui_kittest::Harness<'_, SlideApp>, name: &str) {
            h.run_steps(3);
            let image = h.render().expect("render");
            let path = std::env::temp_dir().join(format!("localslides-{name}.png"));
            image.save(&path).expect("save");
            println!("{}", path.display());
        }
        for (name, brightness) in [("light", crate::theme::Brightness::Light), ("dark", crate::theme::Brightness::Dark)] {
            let mut a = app(brightness);
            a.ai.chat_open = true;
            a.ai.chat.push_answer("The deck has **two** slides.\n\n- Fotossíntese\n- Luz");
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(draw, a);
            save(&mut h, &format!("ai-chat-{name}"));

            let mut b = app(brightness);
            b.ai.settings_open = true;
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 800.0)).build_ui_state(draw, b);
            save(&mut h, &format!("ai-connections-{name}"));
        }
    }
}
