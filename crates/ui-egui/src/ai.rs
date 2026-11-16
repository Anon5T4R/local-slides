//! The AI surface: connections, the three deck actions, the status strip and the dialogs.
//!
//! Everything the user configures is a [`local_ai::Connection`] (provider preset, base URL, model)
//! plus a key that lives in the OS keychain. An action turns the deck (or one slide) into a
//! [`Request`], the model streams an answer, and [`localslides_ai::actions::interpret`] turns it
//! into an [`Outcome`] that is applied through the engine's own commands inside **one undo group**
//! ([`Session::grouped`]) - so a failed apply rolls back and the `.pptx` is never written by the AI
//! directly.
//!
//! The AI is desktop-only: `local-ai` reads the OS keychain and speaks HTTP, which the web build
//! does not have.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use egui::{Context, RichText, Ui};
use localslides_ai::local_ai::{self, Connection};
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

/// A connection being edited (new or existing).
pub struct Draft {
    pub id: String,
    pub provider: String,
    pub label: String,
    pub base_url: String,
    pub model: String,
    pub key: String,
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
    pub connections: Vec<Connection>,
    pub active: Option<String>,
    pub keys: HashMap<String, bool>,
    keys_loaded: bool,
    /// The prompts' language (the system's, read once).
    language: String,
    pub panel: bool,
    pub dialog: Option<Dialog>,
    pub draft: Option<Draft>,
    pub models: Option<Vec<String>>,
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
        let connections = match Connection::load_all(&app) {
            Ok(connections) => connections,
            Err(e) => {
                error = Some(format!("the AI connections could not be read: {e}"));
                Vec::new()
            }
        };
        let active = connections.first().map(|connection| connection.id.clone());
        Self {
            backend: Arc::new(LiveBackend::new(AI_APP_ID, AI_APP_URL)),
            connections,
            active,
            keys: HashMap::new(),
            keys_loaded: false,
            language: sys_locale::get_locale().unwrap_or_else(|| "en".to_string()),
            panel: false,
            dialog: None,
            draft: None,
            models: None,
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

    fn active_connection(&self) -> Option<&Connection> {
        self.active.as_ref().and_then(|id| self.connections.iter().find(|connection| &connection.id == id))
    }

    /// Re-reads the keychain for every connection (the panel calls this once when it opens).
    fn refresh_keys(&mut self) {
        let app = Self::app();
        self.keys.clear();
        for connection in &self.connections {
            self.keys.insert(connection.id.clone(), connection.has_key(&app));
        }
        self.keys_loaded = true;
    }

    /// Starts one request on a worker thread. The prompts follow the system's language.
    pub(crate) fn start(&mut self, ctx: &Context, request: Request, apply: Apply) -> Result<(), String> {
        let Some(connection) = self.active_connection().cloned() else {
            return Err("No AI connection yet. Open AI > Connections and add one.".into());
        };
        let messages = localslides_ai::actions::messages(&request, &self.language)?;
        // Remember the request so the dialog can come back pre-filled (repeat).
        if let (Request::GenerateDeck { topic, slides }, Apply::Deck { theme }) = (&request, &apply) {
            self.last = Some(LastDeck { topic: topic.clone(), slides: *slides, theme: theme.clone() });
        }
        let (tx, rx) = mpsc::channel();
        let backend = self.backend.clone();
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

    /// Polls the worker and the model fetch; applies a finished outcome. Returns true when a
    /// command changed the deck (the window then shows the new slide).
    pub(crate) fn tick(&mut self, session: &mut Session) -> bool {
        self.tick_models();
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
                self.models = Some(models);
                self.fetching = None;
            }
            Ok(Err(error)) => {
                self.error = Some(error);
                self.fetching = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.fetching = None,
        }
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
            self.error = Some("Open a presentation first.".into());
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
            self.error = Some("Open a presentation first.".into());
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
            if ui.button("Dismiss").clicked() {
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
        if ui.button("Stop").clicked() {
            stop = true;
        }
    });
    if stop {
        app.ai.stop();
    }
}

/// The connections window and the action dialogs, drawn last so they sit above the deck.
pub(crate) fn windows(app: &mut SlideApp, ctx: &Context) {
    connections_window(app, ctx);
    dialog_window(app, ctx);
}

fn connections_window(app: &mut SlideApp, ctx: &Context) {
    if !app.ai.panel {
        return;
    }
    if !app.ai.keys_loaded {
        app.ai.refresh_keys();
    }
    let mut open = true;
    let mut add = false;
    let mut edit: Option<Connection> = None;
    let mut delete: Option<String> = None;
    let mut activate: Option<String> = None;
    let mut save = false;
    let mut fetch = false;
    let mut close_draft = false;
    egui::Window::new("AI connections")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(540.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label(RichText::new("The key lives in the OS keychain; the connection file never holds it.").weak());
            ui.add_space(4.0);
            if app.ai.connections.is_empty() {
                ui.label("No connections yet.");
            }
            for connection in &app.ai.connections {
                ui.horizontal(|ui| {
                    let active = app.ai.active.as_deref() == Some(connection.id.as_str());
                    if ui.selectable_label(active, format!("{} · {} · {}", connection.label, connection.provider, connection.model)).clicked() {
                        activate = Some(connection.id.clone());
                    }
                    if app.ai.keys.get(&connection.id).copied().unwrap_or(false) {
                        ui.label(RichText::new("key saved").weak());
                    }
                    if ui.small_button("Edit").clicked() {
                        edit = Some(connection.clone());
                    }
                    if ui.small_button("Delete").clicked() {
                        delete = Some(connection.id.clone());
                    }
                });
            }
            ui.add_space(6.0);
            if ui.button("Add connection").clicked() {
                add = true;
            }
            if let Some(draft) = app.ai.draft.as_mut() {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Provider");
                    craft_ui::ComboBox::from_id_salt("ai-provider").selected_text(draft.provider.clone()).show_ui(ui, |ui| {
                        for provider in local_ai::PROVIDERS {
                            if ui.selectable_label(draft.provider == provider.id, provider.label).clicked() {
                                draft.provider = provider.id.to_string();
                                if !provider.base_url.is_empty() {
                                    draft.base_url = provider.base_url.to_string();
                                }
                                if draft.label.is_empty() {
                                    draft.label = provider.label.to_string();
                                }
                            }
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label("Label");
                    ui.text_edit_singleline(&mut draft.label);
                });
                ui.horizontal(|ui| {
                    ui.label("Base URL");
                    ui.text_edit_singleline(&mut draft.base_url);
                });
                ui.horizontal(|ui| {
                    ui.label("Model");
                    if let Some(models) = &app.ai.models {
                        craft_ui::ComboBox::from_id_salt("ai-model").selected_text(draft.model.clone()).show_ui(ui, |ui| {
                            for model in models {
                                if ui.selectable_label(draft.model == *model, model).clicked() {
                                    draft.model = model.clone();
                                }
                            }
                        });
                    } else {
                        ui.text_edit_singleline(&mut draft.model);
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Fetch models").clicked() {
                        fetch = true;
                    }
                    if app.ai.fetching.is_some() {
                        ui.spinner();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("API key");
                    ui.add(egui::TextEdit::singleline(&mut draft.key).password(true));
                });
                ui.horizontal(|ui| {
                    if ui.button("Save connection").clicked() {
                        save = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close_draft = true;
                    }
                });
            }
        });
    app.ai.panel = open;
    let app_id = Ai::app();
    if add {
        let provider = local_ai::PROVIDERS.iter().find(|provider| provider.id == "openai").or_else(|| local_ai::PROVIDERS.first());
        let (id, base, label) = provider.map(|provider| (provider.id, provider.base_url, provider.label)).unwrap_or(("custom", "", "Custom"));
        app.ai.draft = Some(Draft {
            id: format!("conn-{}", crate::clock::now_ms() as u64),
            provider: id.to_string(),
            label: label.to_string(),
            base_url: base.to_string(),
            model: String::new(),
            key: String::new(),
        });
        app.ai.models = None;
    }
    if let Some(connection) = edit {
        app.ai.draft = Some(Draft {
            id: connection.id.clone(),
            provider: connection.provider.clone(),
            label: connection.label.clone(),
            base_url: connection.base_url.clone(),
            model: connection.model.clone(),
            key: String::new(),
        });
        app.ai.models = None;
    }
    if let Some(id) = delete
        && let Some(connection) = app.ai.connections.iter().find(|connection| connection.id == id).cloned()
    {
        let _ = connection.clear_key(&app_id);
        app.ai.connections.retain(|connection| connection.id != id);
        let _ = Connection::save_all(&app_id, &app.ai.connections);
        app.ai.keys.remove(&id);
        if app.ai.active.as_deref() == Some(id.as_str()) {
            app.ai.active = app.ai.connections.first().map(|connection| connection.id.clone());
        }
    }
    if let Some(id) = activate {
        app.ai.active = Some(id);
    }
    if fetch && let Some(draft) = &app.ai.draft {
        let connection = Connection {
            id: draft.id.clone(),
            provider: draft.provider.clone(),
            label: draft.label.clone(),
            base_url: draft.base_url.clone(),
            model: draft.model.clone(),
        };
        let key = if draft.key.is_empty() { connection.key(&app_id).unwrap_or_default() } else { draft.key.clone() };
        let (tx, rx) = mpsc::channel();
        let worker = ctx.clone();
        std::thread::spawn(move || {
            let app = local_ai::App { id: AI_APP_ID, url: AI_APP_URL };
            let result = local_ai::list_models_with_key(&app, &connection, &key);
            let _ = tx.send(result);
            worker.request_repaint();
        });
        app.ai.fetching = Some(rx);
    }
    if close_draft {
        app.ai.draft = None;
        app.ai.models = None;
    }
    if save && let Some(draft) = app.ai.draft.take() {
        let connection = Connection {
            id: draft.id.clone(),
            provider: draft.provider.clone(),
            label: draft.label.clone(),
            base_url: draft.base_url.clone(),
            model: draft.model.clone(),
        };
        if !draft.key.is_empty()
            && let Err(error) = connection.set_key(&app_id, &draft.key)
        {
            app.ai.error = Some(format!("the key could not be saved: {error}"));
        }
        if let Some(existing) = app.ai.connections.iter_mut().find(|connection| connection.id == draft.id) {
            *existing = connection;
        } else {
            app.ai.connections.push(connection);
        }
        if let Err(error) = Connection::save_all(&app_id, &app.ai.connections) {
            app.ai.error = Some(format!("the AI connections could not be saved: {error}"));
        }
        app.ai.active = app.ai.connections.first().map(|connection| connection.id.clone());
        app.ai.keys_loaded = false;
        app.ai.models = None;
    }
}

fn dialog_window(app: &mut SlideApp, ctx: &Context) {
    let Some(mut dialog) = app.ai.dialog.take() else { return };
    let mut run: Option<(Request, Apply)> = None;
    let mut close = false;
    let mut error: Option<String> = None;
    let title = match &dialog {
        Dialog::Generate { .. } => "Generate presentation",
        Dialog::Edit { .. } => "Rewrite slide",
        Dialog::Fix { .. } => "Correct slides",
    };
    egui::Window::new(title).collapsible(false).resizable(false).default_width(500.0).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(
        ctx,
        |ui| match &mut dialog {
            Dialog::Generate { topic, slides, theme, error: dialog_error } => {
                ui.label("Describe the presentation: a topic, an audience, or a whole text to organize.");
                ui.add(egui::TextEdit::multiline(topic).desired_rows(4).desired_width(f32::INFINITY));
                ui.horizontal(|ui| {
                    ui.label("Slides");
                    ui.add(egui::DragValue::new(slides).range(1..=localslides_ai::actions::MAX_SLIDES));
                });
                // The identity preset: the engine's own themes, "None" first (opt-in).
                ui.horizontal(|ui| {
                    ui.label("Theme");
                    if ui.selectable_label(theme.is_none(), "None").clicked() {
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
                    if ui.button("Generate").clicked() {
                        run = Some((Request::GenerateDeck { topic: topic.clone(), slides: *slides }, Apply::Deck { theme: theme.clone() }));
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            }
            Dialog::Edit { index, title, body, instruction, error: dialog_error } => {
                ui.label(format!("Slide {}:", *index + 1));
                ui.label(RichText::new(title.as_str()).monospace());
                ui.label(RichText::new(body.as_str()).monospace().weak().small());
                ui.separator();
                ui.label("Instruction (for example: shorter, translate, reorder the bullets).");
                ui.add(egui::TextEdit::multiline(instruction).desired_rows(3).desired_width(f32::INFINITY));
                if let Some(message) = dialog_error.as_ref() {
                    ui.colored_label(ui.visuals().error_fg_color, message);
                }
                ui.horizontal(|ui| {
                    if ui.button("Rewrite").clicked() {
                        run = Some((
                            Request::EditSlide { index: *index, title: title.clone(), body: body.clone(), instruction: instruction.clone() },
                            Apply::Slide { index: *index },
                        ));
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            }
            Dialog::Fix { all, error: dialog_error } => {
                ui.label("Correct the spelling, grammar and punctuation. The meaning, the length and the language stay.");
                ui.checkbox(all, "All slides (otherwise only this one)");
                if let Some(message) = dialog_error.as_ref() {
                    ui.colored_label(ui.visuals().error_fg_color, message);
                }
                ui.horizontal(|ui| {
                    if ui.button("Correct").clicked() {
                        match collect_slides(&mut app.session, *all) {
                            Ok(slides) => run = Some((Request::FixSlides { slides }, Apply::Fix)),
                            Err(message) => error = Some(message),
                        }
                    }
                    if ui.button("Cancel").clicked() {
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
}
