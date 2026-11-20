//! The AI tab: the three deck actions and the connections panel.

use super::{Icon, SlideApp, Ui, big_button, group};

pub(super) fn ai(app: &mut SlideApp, ui: &mut Ui) {
    let open = app.session.active().is_some();
    group(ui, |ui| {
        if big_button(ui, Icon::Ai, "Generate\nPresentation", open).clicked() {
            app.ai.open_generate(&mut app.session);
        }
    });
    group(ui, |ui| {
        if big_button(ui, Icon::Pencil, "Rewrite\nSlide", open).clicked() {
            app.ai.open_edit(&mut app.session);
        }
    });
    group(ui, |ui| {
        if big_button(ui, Icon::Spelling, "Correct\nText", open).clicked() {
            app.ai.open_fix(&mut app.session);
        }
    });
    group(ui, |ui| {
        if big_button(ui, Icon::Ai, "AI\nAssistant", true).clicked() {
            app.ai.chat_open = !app.ai.chat_open;
        }
    });
    group(ui, |ui| {
        if big_button(ui, Icon::Settings, "Connections", true).clicked() {
            app.ai.settings_open = true;
        }
    });
}
