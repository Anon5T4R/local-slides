//! The AI tab in the window: the three actions are reachable, the generate dialog carries the
//! theme preset and comes back pre-filled, and the connections panel opens.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use localslides_engine::Session;

use crate::ai::{Dialog, LastDeck};
use crate::{Services, SlideApp};

fn harness() -> Harness<'static, SlideApp> {
    let mut app = SlideApp::new(Session::with_new(), Services::default());
    app.ui.tab = "AI".into();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

#[test]
fn the_ai_tab_offers_the_three_actions_and_connections() {
    let h = harness();
    for label in ["Generate Presentation", "Rewrite Slide", "Correct Text", "Connections"] {
        assert!(h.query_by_label(label).is_some(), "{label} is missing from the AI tab");
    }
}

#[test]
fn generate_opens_its_dialog_with_the_engine_themes() {
    let mut h = harness();
    h.get_by_label("Generate Presentation").click();
    h.run_steps(3);
    assert!(matches!(h.state().ai.dialog, Some(Dialog::Generate { .. })), "the generate dialog is open");
    assert!(h.state().ai.themes.len() >= 5, "the identity picker lists the engine's themes: {:?}", h.state().ai.themes);
    assert!(h.query_by_label("Theme").is_some(), "the theme row is painted");
}

/// The suite's "repeat" gesture: the dialog comes back with the last topic, slide count and theme.
#[test]
fn the_generate_dialog_comes_back_prefilled() {
    let mut h = harness();
    h.state_mut().ai.last = Some(LastDeck { topic: "Fotossíntese".into(), slides: 4, theme: Some("Harbor".into()) });
    h.get_by_label("Generate Presentation").click();
    h.run_steps(3);
    match &h.state().ai.dialog {
        Some(Dialog::Generate { topic, slides, theme, .. }) => {
            assert_eq!(topic, "Fotossíntese");
            assert_eq!(*slides, 4);
            assert_eq!(theme.as_deref(), Some("Harbor"));
        }
        _ => panic!("the generate dialog is open"),
    }
}

#[test]
fn connections_opens_the_panel() {
    let mut h = harness();
    h.get_by_label("Connections").click();
    h.run_steps(3);
    assert!(h.state().ai.panel, "the connections panel is open");
    assert!(h.query_by_label("Add connection").is_some(), "the panel offers a new connection");
}
