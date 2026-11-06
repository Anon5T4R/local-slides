//! One native window per open presentation. Each window is an egui viewport, driven by
//! [`craft_window::Viewports`]: one window draws in the root viewport, every other one in an
//! immediate viewport of its own, and each draws the whole interface (title bar, ribbon, slide,
//! status bar, dialogs) for its own document.
//!
//! A frame: [`SlideApp::logic`] does the app-wide work once (the control channel, media clocks,
//! AutoRecover, background opening). [`SlideApp::ui`] then draws the windows in turn: for each it
//! makes the window's presentation the session's active one, swaps the window's own interface
//! state in ([`WindowUi`]), does the window's work and draws it. Last, the active window's state
//! comes back.
//!
//! Which window shows what follows the session ([`SlideApp::sync_document`]): a presentation made
//! active that no window shows (New, Open) gets a new window, focused and cascaded from the last
//! one; a presentation a window shows brings that window to the front; a window whose presentation
//! closed closes. Opening a file replaces an untouched blank presentation in the window it was
//! opened from. The last window stays open with no presentation, showing the start screen. The web
//! app has one window and switches presentations in it.
//!
//! View › New Window opens another editable window on a presentation. The presentation, its undo
//! history and its save state are shared; its selection is already per document in the session, so
//! its windows share that view too. What the app keeps per window (`WindowUi`) is the interface
//! state another window must not inherit: its File page, open dialogs and messages, its ribbon
//! tab, its slide show, its eyedropper and slide drag. Panes, zoom and the Quick Access Toolbar
//! are preferences and stay shared.
//!
//! A slide show in a window that is not the active one keeps running: [`SlideApp::logic`] advances
//! the clocks of every open window, not only the one whose state is in the app's fields.
//!
//! Closing one of a presentation's windows leaves the presentation open and asks nothing; closing
//! its last window closes it, asking about unsaved changes first.

use std::collections::{HashMap, HashSet};

use craft_window::{Document, Frame, Viewports, WindowEvent, WindowId, WindowList, WindowMenu};
use egui::{ViewportCommand, ViewportId};
use serde_json::{Value, json};

use crate::{AnimPreview, FileTarget, SlideApp, backstage, dialogs, eyedropper, notes, show, thumbs};

/// The application's name: the window title's suffix and what an empty window is called.
pub(crate) const APP_NAME: &str = "Local Slides";

/// The window title for a presentation called `name`.
pub(crate) fn title(name: &str, dirty: bool) -> String {
    format!("{name}{} - {APP_NAME}", if dirty { " •" } else { "" })
}

/// A window's interface state while another window's is in [`SlideApp`]'s fields.
///
/// Everything here is owned by the window that opened it: a dialog, the File page, a status
/// message or a running slide show belongs to one window, not to the app. The presentation's own
/// state (selection, zoom mode, notes, panes) is the session's or [`crate::UiState`]'s and stays
/// shared.
pub(crate) struct WindowUi {
    pub(crate) dialog: Option<dialogs::Dialog>,
    pub(crate) backstage: Option<backstage::Backstage>,
    pub(crate) status: Option<(String, f64)>,
    pub(crate) toast: Option<crate::toast::Toast>,
    pub(crate) palette: Option<(String, usize)>,
    /// The active ribbon tab. The one piece of [`crate::UiState`] a window owns: two windows side
    /// by side can look at different tabs, while zoom, panes, the ruler and the Quick Access
    /// Toolbar stay preferences of the app.
    pub(crate) tab: String,
    pub(crate) show: Option<show::Show>,
    pub(crate) eyedropper: Option<eyedropper::State>,
    pub(crate) reuse_target: Option<FileTarget>,
    pub(crate) thesaurus_query: String,
    pub(crate) anim_preview: Option<AnimPreview>,
    pub(crate) thumb_drag: Option<thumbs::SlideDrag>,
    pub(crate) slides_focused: bool,
    pub(crate) notes_buf: (u64, usize, notes::Draft),
    pub(crate) align_to: Option<(Vec<localslides_model::ShapeId>, bool)>,
    /// The canvas geometry this window's last pass laid out, for the control channel's pointer
    /// conversion: each window maps window points with its own.
    pub(crate) canvas_rect: Option<egui::Rect>,
    pub(crate) canvas_scale: f32,
    pub(crate) slide_rect: Option<egui::Rect>,
    pub(crate) canvas_region: Option<egui::Rect>,
}

impl Default for WindowUi {
    fn default() -> Self {
        WindowUi {
            dialog: None,
            backstage: None,
            status: None,
            toast: None,
            palette: None,
            tab: crate::UiState::default().tab,
            show: None,
            eyedropper: None,
            reuse_target: None,
            thesaurus_query: String::new(),
            anim_preview: None,
            thumb_drag: None,
            slides_focused: false,
            notes_buf: (0, usize::MAX, notes::Draft::default()),
            align_to: None,
            canvas_rect: None,
            canvas_scale: 1.0,
            slide_rect: None,
            canvas_region: None,
        }
    }
}

impl WindowUi {
    /// Put the drawn window's interface state aside.
    pub(crate) fn take_from(app: &mut SlideApp) -> WindowUi {
        WindowUi {
            dialog: app.dialog.take(),
            backstage: app.backstage.take(),
            status: app.status.take(),
            toast: app.toast.take(),
            palette: app.palette.take(),
            tab: std::mem::take(&mut app.ui.tab),
            show: app.show.take(),
            eyedropper: app.eyedropper.take(),
            reuse_target: app.reuse_target.take(),
            thesaurus_query: std::mem::take(&mut app.thesaurus_query),
            anim_preview: app.anim_preview.take(),
            thumb_drag: app.thumb_drag.take(),
            slides_focused: std::mem::replace(&mut app.slides_focused, false),
            notes_buf: std::mem::replace(&mut app.notes_buf, (0, usize::MAX, notes::Draft::default())),
            align_to: app.align_to.take(),
            canvas_rect: app.canvas_rect.take(),
            canvas_scale: std::mem::replace(&mut app.canvas_scale, 1.0),
            slide_rect: app.slide_rect.take(),
            canvas_region: app.canvas_region.take(),
        }
    }

    /// Bring this window's interface state back into the drawn window's fields.
    pub(crate) fn put_into(self, app: &mut SlideApp) {
        app.dialog = self.dialog;
        app.backstage = self.backstage;
        app.status = self.status;
        app.toast = self.toast;
        app.palette = self.palette;
        app.ui.tab = self.tab;
        app.show = self.show;
        app.eyedropper = self.eyedropper;
        app.reuse_target = self.reuse_target;
        app.thesaurus_query = self.thesaurus_query;
        app.anim_preview = self.anim_preview;
        app.thumb_drag = self.thumb_drag;
        app.slides_focused = self.slides_focused;
        app.notes_buf = self.notes_buf;
        app.align_to = self.align_to;
        app.canvas_rect = self.canvas_rect;
        app.canvas_scale = self.canvas_scale;
        app.slide_rect = self.slide_rect;
        app.canvas_region = self.canvas_region;
    }
}

/// An open window.
pub(crate) struct Window {
    /// The app's key for the window. A window made while the windows are being drawn gets its
    /// [`WindowId`] once the frame's drawing ends.
    key: u64,
    id: Option<WindowId>,
    /// The presentation it shows; `None` for the start screen's empty window.
    doc: Option<u64>,
    /// Its interface state while another window's is in [`SlideApp`]'s fields; `None` for the
    /// drawn window.
    ui: Option<Box<WindowUi>>,
}

/// The app's windows. See the module documentation.
pub struct Windows {
    /// Separate native windows (desktop). Off, one window shows every presentation in turn (web).
    native: bool,
    /// Every window, with its key, for the viewport driver: opening and focus order, positions.
    /// Lent to the driver while the windows draw.
    list: WindowList<u64>,
    viewports: Option<Viewports>,
    /// Open windows in opening order.
    open: Vec<Window>,
    /// The window whose state is in [`SlideApp`]'s fields.
    shown: u64,
    next_key: u64,
    /// The windows are being drawn: `list` is lent out, so windows opened or closed now join or
    /// leave it when the drawing ends.
    showing: bool,
    /// Window keys, most recently focused first, as the drawing started.
    recent: Vec<u64>,
    /// A window to bring to the front when the drawing ends.
    focus: Option<u64>,
    /// View › Window › Switch Windows and the window titles, as the frame's drawing started.
    menu: WindowMenu,
    /// What a new window's viewport starts from: size, icon, app id.
    builder: egui::ViewportBuilder,
    /// The viewport driver draws the windows this frame.
    driven: bool,
    /// The window being drawn and the viewport it draws in, while [`SlideApp::ui`] runs for it.
    drawing: Option<(u64, ViewportId)>,
    /// Windows whose slide show advanced in the last frame's drawing: their own pass ran the
    /// clocks, so the next logic pass must not run them a second time.
    drawn_shows: HashSet<u64>,
    /// [`ViewportInput`] is registered with egui.
    input_plugin: bool,
}

impl Windows {
    /// One empty window: the start screen. It is the root viewport's window.
    pub(crate) fn new() -> Windows {
        let mut list = WindowList::new();
        let id = list.open(1);
        let mut viewports = Viewports::new(Frame::Desktop);
        viewports.set_root(Some(id));
        Windows {
            native: cfg!(not(target_arch = "wasm32")),
            list,
            viewports: Some(viewports),
            open: vec![Window { key: 1, id: Some(id), doc: None, ui: None }],
            shown: 1,
            next_key: 2,
            showing: false,
            recent: vec![1],
            focus: None,
            menu: WindowMenu::default(),
            builder: egui::ViewportBuilder::default().with_inner_size([1440.0, 900.0]).with_min_inner_size([900.0, 560.0]),
            driven: false,
            drawing: None,
            drawn_shows: HashSet::new(),
            input_plugin: false,
        }
    }

    /// Whether presentations get native windows of their own.
    pub(crate) fn native(&self) -> bool {
        self.native
    }

    /// One window for every presentation, as in the web app.
    #[cfg(test)]
    pub(crate) fn single(&mut self) {
        self.native = false;
    }

    /// The root window shows presentation `doc` from the start (a session built with one open).
    pub(crate) fn start_with(&mut self, doc: Option<u64>) {
        if let Some(w) = self.open.iter_mut().find(|w| w.key == self.shown) {
            w.doc = doc;
        }
    }

    /// The key of the window whose state is in [`SlideApp`]'s fields.
    pub(crate) fn shown(&self) -> u64 {
        self.shown
    }

    /// How many windows are open (one, in the web app).
    pub(crate) fn count(&self) -> usize {
        if self.native { self.open.len() } else { 1 }
    }

    fn get(&self, key: u64) -> Option<&Window> {
        self.open.iter().find(|w| w.key == key)
    }

    fn get_mut(&mut self, key: u64) -> Option<&mut Window> {
        self.open.iter_mut().find(|w| w.key == key)
    }

    /// The presentation a window shows.
    pub(crate) fn doc_of(&self, key: u64) -> Option<u64> {
        self.get(key).and_then(|w| w.doc)
    }

    /// How many windows show presentation `doc`.
    fn windows_on(&self, doc: u64) -> usize {
        self.open.iter().filter(|w| w.doc == Some(doc)).count()
    }

    /// The window showing presentation `doc`, the most recently focused if several do.
    fn of_doc(&self, doc: u64) -> Option<u64> {
        self.recent_keys().into_iter().find(|&k| self.doc_of(k) == Some(doc))
    }

    /// Open windows, most recently focused first; windows opened during this frame's drawing
    /// come first.
    fn recent_keys(&self) -> Vec<u64> {
        let mut keys: Vec<u64> = self.open.iter().rev().filter(|w| w.id.is_none()).map(|w| w.key).collect();
        let listed: Vec<u64> =
            if self.showing { self.recent.clone() } else { self.list.recent().filter_map(|id| self.list.get(id).copied()).collect() };
        keys.extend(listed.into_iter().filter(|&k| self.get(k).is_some()));
        for w in &self.open {
            if !keys.contains(&w.key) {
                keys.push(w.key);
            }
        }
        keys
    }

    /// The open window keys in opening order.
    pub(crate) fn keys_in_order(&self) -> Vec<u64> {
        self.open.iter().map(|w| w.key).collect()
    }

    /// A parked window's interface state; `None` for the drawn window, whose state is in
    /// [`SlideApp`]'s fields, and for a key that is not open.
    pub(crate) fn parked_ui_mut(&mut self, key: u64) -> Option<&mut WindowUi> {
        self.get_mut(key).and_then(|w| w.ui.as_deref_mut())
    }

    /// Note that window `key`'s slide show advanced in this frame's drawing.
    pub(crate) fn mark_show_drawn(&mut self, key: u64) {
        self.drawn_shows.insert(key);
    }

    /// The windows whose show advanced in the last frame's drawing, clearing the record.
    pub(crate) fn take_drawn_shows(&mut self) -> HashSet<u64> {
        std::mem::take(&mut self.drawn_shows)
    }

    /// The egui viewport a window draws in.
    pub(crate) fn viewport_of(&self, key: u64) -> ViewportId {
        self.viewport(key)
    }

    /// The window's position in opening order, as View › Switch Windows numbers it.
    fn index_of(&self, key: u64) -> Option<usize> {
        self.open.iter().position(|w| w.key == key)
    }

    /// A fresh key for a new window.
    fn mint_key(&mut self) -> u64 {
        let key = self.next_key;
        self.next_key = self.next_key.saturating_add(1);
        key
    }

    /// The window's viewport.
    fn viewport(&self, key: u64) -> ViewportId {
        match (self.get(key).and_then(|w| w.id), &self.viewports) {
            (Some(id), Some(v)) => v.viewport_id(id),
            _ => ViewportId::ROOT,
        }
    }

    /// The title View › Window › Switch Windows gives the window, as the frame started.
    fn menu_title(&self, key: u64) -> Option<&str> {
        self.menu.title(self.get(key)?.id?)
    }

    /// Bring the list up to date with the windows opened, closed and focused since it was last
    /// lent out, and give the root viewport a window when its own closed.
    fn settle(&mut self, ctx: Option<&egui::Context>) {
        if self.showing {
            return;
        }
        let gone: Vec<WindowId> = self.list.ids().filter(|id| !self.open.iter().any(|w| w.id == Some(*id))).collect();
        for id in gone {
            self.list.close(id);
        }
        let active = self.list.active();
        let mut opened = false;
        for w in self.open.iter_mut().filter(|w| w.id.is_none()) {
            w.id = Some(self.list.open(w.key));
            opened = true;
        }
        let focus = self.focus.take().and_then(|k| self.get(k)).and_then(|w| w.id);
        // A window opened behind the others (nothing asked to focus it) leaves the active one so.
        if let Some(id) = focus.or(active.filter(|_| opened)) {
            self.list.focus(id);
        }
        let Some(viewports) = self.viewports.as_mut() else { return };
        if !viewports.root().is_some_and(|r| self.list.contains(r))
            && let Some(next) = self.list.active()
        {
            match ctx {
                Some(ctx) => viewports.adopt(ctx, &self.list, next),
                None => viewports.set_root(Some(next)),
            }
        }
        if let (Some(id), Some(ctx)) = (focus, ctx) {
            viewports.focus(ctx, id);
        }
        if opened && let Some(ctx) = ctx {
            ctx.request_repaint();
        }
    }
}

impl SlideApp {
    /// How many windows are open.
    pub fn window_count(&self) -> usize {
        self.windows.count()
    }

    /// The window being drawn and the viewport it draws in, for the admission check in
    /// [`Self::ui_window`].
    pub(crate) fn drawing_viewport(&self) -> Option<ViewportId> {
        self.windows.drawing.map(|(_, viewport)| viewport)
    }

    /// The egui viewport of the active window, the one focused last: control input,
    /// `ui.screenshot`, `ui.resize` and the saved window geometry are its. The root viewport
    /// when the windows don't have viewports of their own (the web app, embedded viewports).
    pub fn active_viewport(&self) -> ViewportId {
        if !self.windows.native || !self.windows.driven {
            return ViewportId::ROOT;
        }
        let key = self.windows.recent_keys().first().copied().unwrap_or(self.windows.shown);
        self.windows.viewport(key)
    }

    /// The active window's viewport and its logical size, for the control channel's output.
    pub(crate) fn active_window_size(&self, ctx: &egui::Context) -> egui::Vec2 {
        let viewport = self.active_viewport();
        if viewport == ViewportId::ROOT {
            return ctx.content_rect().size();
        }
        ctx.input_for(viewport, |i| i.viewport().inner_rect.map(|r| r.size()).unwrap_or_else(|| ctx.content_rect().size()))
    }

    /// The viewport new windows start from: the host's size, icon and app id. Their title and
    /// place are the app's.
    pub fn set_window_builder(&mut self, builder: egui::ViewportBuilder) {
        self.windows.builder = builder;
    }

    /// Whether a close request for the root viewport closes only the window drawn there: true
    /// while other windows are open. The quit prompt in `logic` asks only for the last one.
    pub fn closes_a_window(&self) -> bool {
        self.window_count() > 1 && !self.quit_confirmed
    }

    /// Put the drawn window's state away and draw window `key`, with its presentation active.
    /// False when there is no such window.
    pub(crate) fn enter_window(&mut self, key: u64) -> bool {
        if key == self.windows.shown {
            self.activate_document_of(key);
            return true;
        }
        if self.windows.get(key).is_none() {
            return false;
        }
        let state = self.windows.get_mut(key).and_then(|w| w.ui.take()).unwrap_or_default();
        let left = self.windows.shown;
        let leaving = WindowUi::take_from(self);
        if let Some(w) = self.windows.get_mut(left) {
            w.ui = Some(Box::new(leaving));
        }
        self.windows.shown = key;
        state.put_into(self);
        self.activate_document_of(key);
        true
    }

    /// Make the presentation window `key` shows the session's active one.
    fn activate_document_of(&mut self, key: u64) {
        let doc = self.windows.doc_of(key);
        if let Some(uid) = doc
            && let Some(i) = self.session.documents().iter().position(|d| d.uid == uid)
        {
            self.session.set_active(i);
        }
        self.shown_document = doc;
    }

    /// Open a new window for open presentation `doc`, focused and cascaded from the last one.
    fn open_window(&mut self, doc: u64) {
        let key = self.windows.mint_key();
        // A new window starts afresh: no dialog, no File page, no status message from the one it
        // was opened from.
        self.windows.open.push(Window { key, id: None, doc: Some(doc), ui: Some(Box::new(WindowUi::default())) });
        self.enter_window(key);
        self.focus_window(key);
    }

    /// View › New Window: another editable window on the drawn window's presentation. Its
    /// selection is the session's (per document), so its windows share it. In one window (the web
    /// app) and with no presentation open, nothing happens.
    pub(crate) fn new_window(&mut self) {
        if !self.windows.native {
            return;
        }
        let Some(uid) = self.session.active().map(|d| d.uid) else { return };
        let key = self.windows.mint_key();
        self.windows.open.push(Window { key, id: None, doc: Some(uid), ui: Some(Box::new(WindowUi::default())) });
        self.enter_window(key);
        self.focus_window(key);
    }

    /// `view.newWindow`. `{on: false}` closes the extra windows on the drawn presentation.
    pub(crate) fn new_window_command(&mut self, p: &Value) -> Result<Value, String> {
        if !self.windows.native {
            return Err("New Window needs the desktop app".into());
        }
        let uid = self.session.active().map(|d| d.uid).ok_or("New Window needs an open presentation")?;
        if p.get("on").and_then(Value::as_bool) == Some(false) {
            let shown = self.windows.shown;
            let extras: Vec<u64> = self.windows.open.iter().filter(|w| w.key != shown && w.doc == Some(uid)).map(|w| w.key).collect();
            for key in extras {
                self.remove_window(key);
            }
            return Ok(json!({"open": self.windows.windows_on(uid) > 1}));
        }
        self.new_window();
        Ok(json!({"open": true, "windows": self.windows.windows_on(uid), "title": self.window_title()}))
    }

    /// The window focused before the drawn one if it shows the same presentation, which then
    /// stays open when the drawn window closes.
    fn other_window_on_document(&self) -> Option<u64> {
        let shown = self.windows.shown;
        let doc = self.windows.doc_of(shown)?;
        if !self.windows.native || self.windows.windows_on(doc) < 2 {
            return None;
        }
        self.windows.recent_keys().into_iter().find(|&k| k != shown)
    }

    /// Remove a window. When it is the drawn one, the window focused before it comes forward and
    /// brings its presentation with it; the root viewport adopts it when the removed window was
    /// the root's. Removing the last window leaves the start screen's empty window.
    pub(crate) fn remove_window(&mut self, key: u64) {
        let (was_shown, doc, played) = {
            let Some(removed) = self.windows.get(key) else { return };
            let was_shown = self.windows.shown == key;
            // A slide show's voices belong to the show, not to the presentation: they stop with
            // the window that plays it, unless the presentation stays open in another window
            // (where another show of it may be using them).
            let played = if was_shown { self.show.is_some() } else { removed.ui.as_ref().is_some_and(|ui| ui.show.is_some()) };
            (was_shown, removed.doc, played)
        };
        if played && doc.is_none_or(|uid| self.windows.windows_on(uid) <= 1) {
            self.media.stop_owned(crate::media::Owner::Show);
        }
        if !was_shown {
            self.windows.open.retain(|w| w.key != key);
            self.settle_windows();
            return;
        }
        // Discard the removed window's interface state and bring the next window's in.
        let next = self.windows.recent_keys().into_iter().find(|&k| k != key);
        self.windows.open.retain(|w| w.key != key);
        match next {
            Some(next) => {
                self.windows.shown = next;
                let state = self.windows.get_mut(next).and_then(|w| w.ui.take()).unwrap_or_default();
                state.put_into(self);
                self.activate_document_of(next);
            }
            None => {
                // The last window stays open with no presentation: the start screen.
                let fresh = self.windows.mint_key();
                self.windows.open.push(Window { key: fresh, id: None, doc: None, ui: None });
                self.windows.shown = fresh;
                WindowUi::default().put_into(self);
                self.shown_document = None;
            }
        }
        self.settle_windows();
    }

    /// Bring window `key` to the front (now, or once the frame's drawing ends) and make it the
    /// active one.
    pub(crate) fn focus_window(&mut self, key: u64) {
        self.windows.focus = Some(key);
        self.settle_windows();
    }

    /// [`Windows::settle`].
    fn settle_windows(&mut self) {
        self.windows.settle(self.ctx.as_ref());
    }

    /// Bring window `key` to the front and make it the active one, with its presentation.
    pub(crate) fn show_window(&mut self, key: u64) {
        if self.enter_window(key) {
            self.focus_window(key);
        }
    }

    /// `window.next`: the next window in opening order, wrapping; `{index}` picks one. The name
    /// stays "Next Window" for the control channel, which now walks windows.
    pub(crate) fn next_window(&mut self, p: &Value) -> Result<Value, String> {
        if !self.windows.native {
            // One window (the web app): the command switches its presentations, as it always did.
            let r = self.session.execute("window.next", p).map_err(|e| e.to_string())?;
            self.sync_document();
            return Ok(r);
        }
        let keys = self.windows.keys_in_order();
        if keys.is_empty() {
            return Ok(Value::Null);
        }
        let key = match p.get("index").and_then(Value::as_u64) {
            Some(i) => *keys.get(usize::try_from(i).unwrap_or(usize::MAX)).ok_or_else(|| format!("no window {i}"))?,
            None => {
                let at = keys.iter().position(|k| *k == self.windows.shown).unwrap_or(keys.len() - 1);
                keys[(at + 1) % keys.len()]
            }
        };
        self.show_window(key);
        self.sync_document();
        Ok(json!({
            "window": self.windows.index_of(key),
            "windows": keys.len(),
            "title": self.window_title(),
        }))
    }

    /// The drawn window's title: its presentation's name, numbered by View › Window › Switch
    /// Windows when several windows show it.
    pub(crate) fn window_title(&self) -> String {
        self.window_title_for(self.windows.shown)
    }

    /// The title window `key` gets: `"{name}{ • if unsaved} - Local Slides"`, the Start screen's
    /// name when it shows no presentation.
    fn window_title_for(&self, key: u64) -> String {
        let Some(uid) = self.windows.doc_of(key) else { return APP_NAME.to_string() };
        let Some(d) = self.session.documents().iter().find(|d| d.uid == uid) else { return APP_NAME.to_string() };
        let name = self.windows.menu_title(key).filter(|t| !t.is_empty()).map(str::to_string).unwrap_or_else(|| d.title());
        title(&name, d.is_dirty())
    }

    /// Whether presentation `uid` is open.
    fn doc_is_open(&self, uid: u64) -> bool {
        self.session.documents().iter().any(|d| d.uid == uid)
    }

    /// An open presentation that has not been used: a new, unsaved, unedited one. Opening a file
    /// takes such a presentation's place instead of opening beside it.
    fn untouched_blank(&self, uid: u64) -> bool {
        self.session.documents().iter().find(|d| d.uid == uid).is_some_and(|d| d.path.is_none() && !d.is_dirty() && d.history.undo.is_empty())
    }

    /// The drawn window and its presentation when an Open would replace it: an untouched blank
    /// document nobody else shows.
    pub(crate) fn open_reuse_target(&self) -> Option<(u64, u64)> {
        if !self.windows.native {
            return None;
        }
        let key = self.windows.shown;
        let uid = self.windows.doc_of(key)?;
        let active = self.session.active()?.uid;
        (uid == active && self.windows.windows_on(uid) == 1 && self.untouched_blank(uid)).then_some((key, uid))
    }

    /// The presentation an Open made active takes the place of `old` in window `key`, and `old`
    /// closes: the window it was opened from is not duplicated.
    pub(crate) fn reuse_blank_window(&mut self, key: (u64, u64), after: u64) {
        let (key, old) = key;
        if after == old || !self.untouched_blank(old) || self.windows.windows_on(old) != 1 {
            return;
        }
        if self.windows.get(key).is_none() {
            return;
        }
        if let Some(w) = self.windows.get_mut(key) {
            w.doc = Some(after);
        }
        if let Some(i) = self.session.documents().iter().position(|d| d.uid == old) {
            self.session.close_document(i);
        }
        // Closing adjusts the active index; the file just opened is the one to show.
        if let Some(i) = self.session.documents().iter().position(|d| d.uid == after) {
            self.session.set_active(i);
        }
        if self.windows.shown == key {
            self.shown_document = Some(after);
        }
    }

    /// The windows follow the session's active presentation after an Open: the file replaces an
    /// untouched blank presentation in `reuse`'s window, else it gets a window of its own.
    pub(crate) fn opened_document(&mut self, reuse: Option<(u64, u64)>) {
        if let (Some(reuse), Some(after)) = (reuse, self.session.active().map(|d| d.uid)) {
            self.reuse_blank_window(reuse, after);
        }
        self.sync_document();
    }

    /// A window whose presentation closed closes, and the window focused before it comes forward.
    /// Documents with no window get one behind the others.
    fn prune_windows(&mut self) {
        let shown = self.windows.shown;
        let open: HashSet<u64> = self.session.documents().iter().map(|d| d.uid).collect();
        self.windows.open.retain(|w| w.key == shown || w.doc.is_none_or(|uid| open.contains(&uid)));
        let active = self.session.active().map(|d| d.uid);
        let mut changed = false;
        for uid in open {
            // The session's active presentation is `sync_document`'s to place; the drawn window's
            // is already shown.
            if Some(uid) == active || self.windows.doc_of(shown) == Some(uid) || self.windows.of_doc(uid).is_some() {
                continue;
            }
            let key = self.windows.mint_key();
            self.windows.open.push(Window { key, id: None, doc: Some(uid), ui: Some(Box::new(WindowUi::default())) });
            changed = true;
        }
        if changed {
            self.settle_windows();
        }
    }

    /// Make the drawn window show `doc` (the session's active presentation, already).
    fn window_shows(&mut self, doc: Option<u64>) {
        let key = self.windows.shown;
        if let Some(w) = self.windows.get_mut(key) {
            w.doc = doc;
        }
        self.shown_document = doc;
    }

    /// Make the session's active presentation the one the drawn window shows, opening or
    /// focusing windows as needed. In one window (the web app) the drawn window switches
    /// presentations.
    pub(crate) fn sync_document(&mut self) {
        if !self.windows.native {
            let active = self.session.active().map(|d| d.uid);
            self.window_shows(active);
            return;
        }
        self.prune_windows();
        let active = self.session.active().map(|d| d.uid);
        let shown = self.windows.shown;
        let shown_doc = self.windows.doc_of(shown);
        match (active, shown_doc) {
            (None, Some(doc)) => {
                if self.doc_is_open(doc) {
                    // The session lost its active document but this one is open: show it.
                    if let Some(i) = self.session.documents().iter().position(|d| d.uid == doc) {
                        self.session.set_active(i);
                    }
                    self.shown_document = Some(doc);
                } else {
                    // Every presentation closed: this window is the start screen again.
                    self.window_shows(None);
                }
            }
            (None, None) => self.shown_document = None,
            (Some(uid), Some(doc)) if doc == uid => self.shown_document = Some(uid),
            (Some(uid), Some(doc)) if !self.doc_is_open(doc) => {
                // The drawn window's presentation closed under it: another window comes forward,
                // or this window takes the active presentation.
                match self.windows.recent_keys().into_iter().find(|&k| k != shown) {
                    Some(next) => {
                        self.show_window(next);
                        self.remove_window(shown);
                    }
                    None => {
                        self.window_shows(Some(uid));
                        self.activate_document_of(shown);
                    }
                }
            }
            (Some(uid), _) => {
                if let Some(key) = self.windows.of_doc(uid).filter(|&k| k != shown) {
                    self.show_window(key);
                } else if shown_doc.is_none() {
                    // The start screen's empty window takes the presentation opened from it.
                    self.window_shows(Some(uid));
                    self.activate_document_of(shown);
                } else {
                    self.open_window(uid);
                }
            }
        }
    }

    /// Draw every window, each in its viewport (see the module documentation).
    pub(crate) fn windows_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.windows.driven = !ctx.embed_viewports() && self.windows.viewports.is_some();
        if !self.windows.driven && ctx.viewport_id() != ViewportId::ROOT {
            // Embedded viewports can only show the active window: a callback for a child one is a
            // render-only imitation (View › New Window, Presenter View) and must not mutate the
            // app's composition state.
            log::debug!("ignoring SlideApp::ui for render-only viewport {:?}", ctx.viewport_id());
            return;
        }
        self.sync_document();
        self.settle_windows();
        self.windows.menu = self.window_menu();
        // A backend that can't open native windows (egui embeds them in the root: tests,
        // `ui_shot`) shows the active window alone. Embedded, every window would draw the same
        // widgets in one viewport.
        if !self.windows.driven {
            let active = self.windows.recent_keys().first().copied().unwrap_or(self.windows.shown);
            self.draw_window(ui, active);
            self.after_windows();
            return;
        }
        let titles: HashMap<u64, String> = self.windows.open.iter().map(|w| (w.key, self.window_title_for(w.key))).collect();
        let Some(mut viewports) = self.windows.viewports.take() else { return };
        let mut list = std::mem::take(&mut self.windows.list);
        self.windows.recent = list.recent().filter_map(|id| list.get(id).copied()).collect();
        let base = self.windows.builder.clone();
        self.windows.showing = true;
        let events = viewports.show(
            ui,
            &mut list,
            |_, key| match titles.get(key) {
                Some(t) => base.clone().with_title(t.clone()),
                None => base.clone().with_title(APP_NAME),
            },
            |ui, _, key| self.draw_window(ui, *key),
        );
        self.windows.showing = false;
        self.windows.list = list;
        self.windows.viewports = Some(viewports);
        self.window_events(&ctx, &events);
        self.after_windows();
    }

    /// After the frame's drawing: windows opened, closed or focused meanwhile settle, and the
    /// active window's state comes back.
    fn after_windows(&mut self) {
        self.prune_windows();
        self.settle_windows();
        if let Some(&active) = self.windows.recent_keys().first() {
            self.enter_window(active);
        }
    }

    /// View › Window › Switch Windows for the open windows.
    fn window_menu(&self) -> WindowMenu {
        let docs = self.session.documents();
        let doc_of: HashMap<u64, Option<u64>> = self.windows.open.iter().map(|w| (w.key, w.doc)).collect();
        WindowMenu::new(&self.windows.list, |_, key| {
            let doc = doc_of.get(key).copied().flatten();
            match doc.and_then(|uid| docs.iter().find(|d| d.uid == uid)) {
                Some(d) => Document { key: doc, name: d.title(), dirty: d.is_dirty() },
                None => Document { key: doc, name: String::new(), dirty: false },
            }
        })
    }

    /// Draw window `key` into its viewport's `ui`.
    fn draw_window(&mut self, ui: &mut egui::Ui, key: u64) {
        if !self.enter_window(key) {
            return;
        }
        let ctx = ui.ctx().clone();
        #[cfg(test)]
        tests::note_drawn(ctx.viewport_id());
        self.windows.drawing = Some((key, ctx.viewport_id()));
        self.sync_backstage();
        self.ui_window(ui);
        self.windows.drawing = None;
        // A visible show pass ran this window's clocks; the next logic pass must not run them
        // again (see `SlideApp::tick_shows`). A discarded pass, or one that moved drawing to
        // another window, advanced nothing.
        if self.windows.shown() == key && self.show.is_some() && show::viewport_visible(&ctx) {
            self.windows.mark_show_drawn(key);
        }
    }

    /// The windows' focus changes and close requests from this frame.
    fn window_events(&mut self, ctx: &egui::Context, events: &[WindowEvent]) {
        for event in events {
            let WindowEvent::CloseRequested(id) = *event else { continue };
            let Some(key) = self.windows.list.get(id).copied() else { continue };
            // The last window's close button quits: the host asks first (the quit prompt in
            // `logic`).
            if self.windows.open.len() <= 1 || self.quit_confirmed {
                continue;
            }
            if self.windows.viewports.as_ref().and_then(Viewports::root) == Some(id) {
                ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::CancelClose);
            }
            self.close_window(key);
        }
    }

    /// The close button of window `key` (not the last one): File › Close in that window, which
    /// asks about unsaved changes first.
    pub(crate) fn close_window(&mut self, key: u64) {
        if self.windows.open.len() <= 1 {
            return;
        }
        if !self.enter_window(key) {
            return;
        }
        self.focus_window(key);
        let Some(uid) = self.windows.doc_of(key) else {
            // The start screen's empty window: nothing to ask about.
            self.remove_window(key);
            return;
        };
        if self.other_window_on_document().is_some() {
            // Another window shows the presentation: only this window closes.
            self.remove_window(key);
            return;
        }
        if self.session.documents().iter().any(|d| d.uid == uid && d.is_dirty()) {
            self.dialog = Some(dialogs::Dialog::new("close"));
            return;
        }
        self.remove_window(key);
        self.close_document_uid(uid);
    }

    /// File › Close and Ctrl+W: closes the window. With one window left the presentation closes
    /// and the window shows the start screen again; Closing the WINDOW itself (its close button)
    /// is what quits the app.
    pub(crate) fn close_presentation(&mut self) {
        if !self.windows.native {
            match self.session.active() {
                None => {}
                Some(d) if d.is_dirty() => self.dialog = Some(dialogs::Dialog::new("close")),
                Some(_) => {
                    let _ = self.run("file.close", json!({}));
                }
            }
            return;
        }
        if self.windows.open.len() > 1 {
            self.close_window(self.windows.shown);
            return;
        }
        // The last window stays open: close its presentation, leaving the start screen.
        let Some(uid) = self.session.active().map(|d| d.uid) else { return };
        if self.session.active().is_some_and(|d| d.is_dirty()) {
            self.dialog = Some(dialogs::Dialog::new("close"));
            return;
        }
        self.close_document_uid(uid);
    }

    /// Close presentation `uid`, then show whichever presentation is active now.
    fn close_document_uid(&mut self, uid: u64) {
        let Some(i) = self.session.documents().iter().position(|d| d.uid == uid) else { return };
        self.session.close_document(i);
        // Keep the drawn window's presentation active: closing an earlier document shifts the
        // index the session would otherwise pick.
        let shown_doc = self.windows.doc_of(self.windows.shown);
        if let Some(doc) = shown_doc
            && self.doc_is_open(doc)
            && let Some(i) = self.session.documents().iter().position(|d| d.uid == doc)
        {
            self.session.set_active(i);
        }
        self.sync_document();
    }

    /// Control-channel input for a window in a viewport of its own: eframe's raw-input hook sees
    /// only the root viewport's input, and egui runs a plugin's input hook for every viewport.
    /// The queued batch goes to the active window's next pass.
    pub(crate) fn send_control_input(&mut self, ctx: &egui::Context) {
        if self.synthetic.is_empty() || self.control_input_is_roots(ctx) {
            return;
        }
        if !self.windows.input_plugin {
            ctx.add_plugin(ViewportInput::default());
            self.windows.input_plugin = true;
        }
        self.settle_windows();
        let target = self.active_viewport();
        // Input queued for a window that has closed since (or moved into the root viewport)
        // would wait forever.
        let live: Vec<ViewportId> = self.windows.open.iter().map(|w| self.windows.viewport(w.key)).filter(|&v| v != ViewportId::ROOT).collect();
        ctx.with_plugin(|p: &mut ViewportInput| {
            if p.target.is_some_and(|t| !live.contains(&t)) {
                p.target = None;
                p.events.clear();
            }
        });
        let waiting = ctx.with_plugin(|p: &mut ViewportInput| !p.events.is_empty()).unwrap_or(true);
        if waiting || self.synthetic.is_empty() {
            return;
        }
        let n = batch_len(&self.synthetic);
        let events: Vec<egui::Event> = self.synthetic.drain(..n).collect();
        ctx.with_plugin(|p: &mut ViewportInput| {
            p.target = Some(target);
            p.events = events;
        });
        // The batch is in flight: later passes must not replay it, and a target change drops it
        // (see `retract_synthetic_after_logic`).
        self.synthetic_pending = true;
        self.synthetic_target = Some(self.input_target());
        self.synthetic_focus = ctx.memory(|m| m.focused());
        self.synthetic_injected.clear();
        self.synthetic_injected_at = None;
        self.synthetic_injected_time = None;
        self.synthetic_viewport = Some(target);
        ctx.request_repaint_of(target);
    }

    /// Whether control input goes in through the root viewport's raw input: one window, the
    /// active window drawn there, or viewports embedded in the root (tests).
    pub(crate) fn control_input_is_roots(&self, ctx: &egui::Context) -> bool {
        !self.windows.native || ctx.embed_viewports() || self.active_viewport() == ViewportId::ROOT
    }
}

/// How many of the queued synthetic events the next batch carries: one pointer event, or up to
/// and including a key release.
pub(crate) fn batch_len(events: &[egui::Event]) -> usize {
    let n = match events.first() {
        Some(egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. }) => 1,
        _ => events.iter().position(|e| matches!(e, egui::Event::Key { pressed: false, .. })).map_or(events.len(), |i| i + 1),
    };
    n.min(events.len())
}

/// Control-channel input for a window in a viewport of its own. eframe's raw-input hook
/// ([`SlideApp::raw_input_hook`]) sees only the root viewport's input, and egui runs a plugin's
/// input hook for every viewport: this gives the queued events to the target viewport's next pass.
#[derive(Default)]
pub(crate) struct ViewportInput {
    /// The viewport the queued events go to, on its next pass.
    target: Option<ViewportId>,
    events: Vec<egui::Event>,
}

impl ViewportInput {
    /// Forget the queued batch: its window closed, or the batch's target changed.
    pub(crate) fn drop_queued(&mut self) {
        self.target = None;
        self.events.clear();
    }
}

impl egui::Plugin for ViewportInput {
    fn debug_name(&self) -> &'static str {
        "localslides_viewport_input"
    }

    fn input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        let viewport = input.viewport_id;
        if viewport == ViewportId::ROOT || self.target != Some(viewport) || self.events.is_empty() {
            return;
        }
        self.target = None;
        let events = std::mem::take(&mut self.events);
        if events.iter().any(|e| matches!(e, egui::Event::Key { .. } | egui::Event::Text(_))) {
            input.focused = true;
        }
        input.events.extend(events);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn the_title_names_the_presentation_and_marks_unsaved_changes() {
        assert_eq!(title("Quarterly review", false), "Quarterly review - Local Slides");
        assert_eq!(title("Quarterly review", true), "Quarterly review • - Local Slides");
    }

    /// A new window joins the driver's list with its own id, in opening order. It leaves the
    /// active window alone until something asks to focus it (its position is the driver's
    /// cascade, tested in `craft_window`).
    #[test]
    fn a_new_window_joins_the_list_in_opening_order() {
        let mut windows = Windows::new();
        assert_eq!(windows.keys_in_order(), vec![1]);
        windows.open.push(Window { key: 2, id: None, doc: None, ui: Some(Box::new(WindowUi::default())) });
        windows.settle(None);
        assert_eq!(windows.keys_in_order(), vec![1, 2]);
        assert!(windows.open.iter().all(|w| w.id.is_some()), "every window got its driver id");
        assert_eq!(windows.recent_keys().first(), Some(&1), "opening behind leaves the active window so");
        windows.focus = Some(2);
        windows.settle(None);
        assert_eq!(windows.recent_keys().first(), Some(&2), "focusing brings it to the front");
        assert_eq!(windows.index_of(2), Some(1));
    }

    /// The start screen's empty window survives with no presentation open.
    #[test]
    fn the_start_screen_window_survives_without_presentations() {
        let mut app = SlideApp::new(localslides_engine::Session::new(), crate::Services::default());
        app.sync_document();
        assert_eq!(app.window_count(), 1);
        assert_eq!(app.windows.doc_of(app.windows.shown()), None);
    }

    thread_local! {
        /// The viewport infos the test backend hands each viewport's pass.
        static INFOS: std::cell::RefCell<egui::ViewportIdMap<egui::ViewportInfo>> =
            std::cell::RefCell::new(egui::ViewportIdMap::default());
        /// The viewports that ran the window UI in the last frame, in drawing order.
        static DRAWN: std::cell::RefCell<Vec<ViewportId>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    /// Record a window's pass for [`DRAWN`].
    pub(crate) fn note_drawn(viewport: ViewportId) {
        DRAWN.with(|d| d.borrow_mut().push(viewport));
    }

    /// A backend with native viewports, as eframe's: each immediate viewport runs a pass of its
    /// own, with its own input.
    fn backend() -> egui::Context {
        INFOS.with(|i| i.borrow_mut().clear());
        DRAWN.with(|d| d.borrow_mut().clear());
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        egui::Context::set_immediate_viewport_renderer(|ctx, viewport| {
            let id = viewport.ids.this;
            let mut raw = egui::RawInput {
                viewport_id: id,
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                ..Default::default()
            };
            raw.viewports = INFOS.with(|i| i.borrow().clone());
            raw.viewports.entry(id).or_default().parent = Some(viewport.ids.parent);
            let mut cb = viewport.viewport_ui_cb;
            let out = ctx.run_ui(raw, |ui| cb(ui));
            out.drop_without_applying_deltas();
        });
        ctx
    }

    /// One frame of the root pass; the immediate viewports run inside it.
    fn frame(ctx: &egui::Context, app: &mut SlideApp) {
        DRAWN.with(|d| d.borrow_mut().clear());
        let mut raw = egui::RawInput { viewports: INFOS.with(|i| i.borrow().clone()), ..Default::default() };
        raw.viewports.entry(ViewportId::ROOT).or_default();
        let out = ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.drop_without_applying_deltas();
        INFOS.with(|i| i.borrow_mut().values_mut().for_each(|v| v.events.clear()));
    }

    /// Each presentation draws in a native window of its own: the first in the root viewport, a
    /// new one in an immediate viewport of its own, and both draw every frame.
    #[test]
    fn each_presentation_draws_in_its_own_viewport() {
        let ctx = backend();
        let mut app = SlideApp::new(localslides_engine::Session::with_new(), crate::Services::default());
        frame(&ctx, &mut app);
        frame(&ctx, &mut app);
        assert_eq!(DRAWN.with(|d| d.borrow().clone()), [ViewportId::ROOT]);
        assert_eq!(app.active_viewport(), ViewportId::ROOT);

        app.run("file.new", json!({})).unwrap();
        frame(&ctx, &mut app);
        let drawn = DRAWN.with(|d| d.borrow().clone());
        assert_eq!(drawn.len(), 2, "both windows draw");
        assert!(drawn.contains(&ViewportId::ROOT), "the first presentation keeps the root viewport");
        let child = app.active_viewport();
        assert_ne!(child, ViewportId::ROOT, "the new presentation has a viewport of its own");
        assert!(drawn.contains(&child));
        assert!(app.window_title().ends_with(" - Local Slides"), "{}", app.window_title());
    }

    /// A pass of the app's logic alone, as a hidden window's frame gives it.
    fn logic_pass(ctx: &egui::Context, app: &mut SlideApp, now: web_time::Instant) {
        let mut raw = egui::RawInput::default();
        raw.viewports.insert(ViewportId::ROOT, egui::ViewportInfo::default());
        let _ = ctx.run_logic(&raw, |ctx| app.tick_shows(ctx, now));
    }

    /// A slide show playing in a window that no longer has the focus keeps its clocks: logic
    /// advances every open window's show, not only the drawn one.
    #[test]
    fn a_show_in_a_window_that_lost_the_focus_keeps_its_clock_running() {
        let ctx = backend();
        let mut app = SlideApp::new(localslides_engine::Session::with_new(), crate::Services::default());
        app.session.execute("slide.new", &json!({})).unwrap();
        app.session.execute("transition.timing", &json!({"index": 0, "after": 1000})).unwrap();
        app.session.execute("transition.timing", &json!({"index": 1, "after": null})).unwrap();
        frame(&ctx, &mut app);
        frame(&ctx, &mut app);

        // A second presentation in a window of its own, its first slide advancing by itself.
        app.run("file.new", json!({})).unwrap();
        app.session.execute("slide.new", &json!({})).unwrap();
        app.session.execute("transition.timing", &json!({"index": 0, "after": 1000})).unwrap();
        app.session.execute("transition.timing", &json!({"index": 1, "after": null})).unwrap();
        app.start_show(0, false);
        frame(&ctx, &mut app);
        frame(&ctx, &mut app);
        let playing = app.windows.shown();
        assert!(app.show.is_some(), "the new window plays the show");
        assert_eq!(app.show.as_ref().map(|s| s.state.slide), Some(0));

        // The first window takes the focus; the show's window stays open in the background.
        let first = app.windows.keys_in_order()[0];
        app.show_window(first);
        assert!(app.show.is_none(), "the drawn window plays no show");
        assert!(app.windows.open.iter().any(|w| w.key == playing && w.ui.as_ref().is_some_and(|ui| ui.show.is_some())));

        // The pass right after the last drawn frame still has that frame's mark; the next one
        // advances the background show from wall time, as it does for a hidden window.
        let anchored = web_time::Instant::now();
        logic_pass(&ctx, &mut app, anchored);
        logic_pass(&ctx, &mut app, anchored + std::time::Duration::from_secs(2));

        let show = app
            .windows
            .open
            .iter()
            .find(|w| w.key == playing)
            .and_then(|w| w.ui.as_ref())
            .and_then(|ui| ui.show.as_ref())
            .expect("the background show is still playing");
        assert_eq!(show.state.slide, 1, "the After: 1 s advance ran with the window in the background");
    }

    /// A 0.5 s sine at 8 kHz, mono 16-bit: a clip `insert.audio` accepts and the media host plays.
    fn wav() -> Vec<u8> {
        let rate = 8000u32;
        let samples: Vec<i16> = (0..rate / 2).map(|i| ((i as f32 * 0.3).sin() * 3000.0) as i16).collect();
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut b = b"RIFF".to_vec();
        b.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        for v in [
            16u32.to_le_bytes().to_vec(),
            1u16.to_le_bytes().to_vec(),
            1u16.to_le_bytes().to_vec(),
            rate.to_le_bytes().to_vec(),
            (rate * 2).to_le_bytes().to_vec(),
            2u16.to_le_bytes().to_vec(),
            16u16.to_le_bytes().to_vec(),
        ] {
            b.extend_from_slice(&v);
        }
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(&data);
        b
    }

    /// A voice playing for a show of the active presentation: audio inserted on slide 0 and
    /// started by the show, which is what a show's media is owned by.
    fn show_playing_audio(app: &mut SlideApp) -> localslides_model::ShapeId {
        use localslides_engine::cmd::base64_encode;
        let inserted = app.session.execute("insert.audio", &json!({"name": "tone.wav", "data": base64_encode(&wav())})).unwrap();
        let id = localslides_model::ShapeId(inserted["id"].as_u64().unwrap() as u32);
        let (uid, doc) = app.session.active().map(|d| (d.uid, d.doc.clone())).unwrap();
        let clip = crate::media::find_clip(&doc, id, Some(0)).unwrap();
        app.start_show(0, false);
        app.media.play(uid, &doc, id, &clip, None, crate::media::Owner::Show);
        assert_eq!(app.media.owner(id), Some(crate::media::Owner::Show));
        id
    }

    /// Closing the window that plays a slide show stops the show's voices: they belong to the
    /// show, not to the presentation.
    #[test]
    fn closing_the_window_of_a_show_stops_its_media() {
        let mut app = SlideApp::new(localslides_engine::Session::with_new(), crate::Services::default());
        // A second presentation, so closing its window closes that presentation's window alone.
        app.run("file.new", json!({})).unwrap();
        let id = show_playing_audio(&mut app);
        let playing = app.windows.shown();
        app.remove_window(playing);
        assert_eq!(app.media.owner(id), None, "the show's voice stopped with its window");
        assert!(app.media.ids().is_empty());
    }

    /// A presentation shown in two windows keeps its show's voices when one of them closes: the
    /// presentation, and its show, continue in the other window.
    #[test]
    fn closing_one_of_the_windows_of_a_presentation_keeps_its_media() {
        let mut app = SlideApp::new(localslides_engine::Session::with_new(), crate::Services::default());
        app.new_window();
        let id = show_playing_audio(&mut app);
        let playing = app.windows.shown();
        app.remove_window(playing);
        assert_eq!(app.media.owner(id), Some(crate::media::Owner::Show), "the presentation is still shown in the other window");
    }

    /// Quitting with a show running stops its voices before the window goes.
    #[test]
    fn quitting_stops_the_shows_media() {
        let mut app = SlideApp::new(localslides_engine::Session::with_new(), crate::Services::default());
        let id = show_playing_audio(&mut app);
        // The quit prompt was answered (Save or Don't Save): the close goes through.
        app.quit_confirmed = true;
        let ctx = egui::Context::default();
        let mut info = egui::ViewportInfo::default();
        info.events.push(egui::ViewportEvent::Close);
        let mut raw = egui::RawInput::default();
        raw.viewports.insert(ViewportId::ROOT, info);
        let _ = ctx.run_logic(&raw, |ctx| app.logic(ctx));
        assert_eq!(app.media.owner(id), None, "quitting stops the show's voices");
    }
}
