//! The suite's shell in the title bar: the theme button, the gear (Settings) and the magnifier
//! (the command palette), plus a visual QA that renders PNGs for review.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use localslides_engine::Session;
use serde_json::json;

use crate::{Services, SlideApp};

fn harness(width: f32, app: SlideApp) -> Harness<'static, SlideApp> {
    let mut h = Harness::builder().with_size(egui::vec2(width, 560.0)).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

fn new_app() -> SlideApp {
    SlideApp::new(Session::with_new(), Services::default())
}

/// The theme button cycles Automatic, Light, Dark and back, and the preference follows.
#[test]
fn title_bar_theme_button_cycles_the_three_modes() {
    let mut h = harness(1100.0, new_app());
    assert!(h.state().ui.system_theme, "a new install follows the system");
    for (now, system, dark) in [("Automatic", false, false), ("Light", false, true), ("Dark", true, true)] {
        h.get_by_label(&format!("Theme: {now}")).click();
        h.run_steps(3);
        assert_eq!(h.state().ui.system_theme, system, "after {now}");
        if !system {
            assert_eq!(h.state().ui.brightness == crate::theme::Brightness::Dark, dark, "after {now}");
        }
    }
}

/// The gear opens Settings, where the theme and the language are.
#[test]
fn title_bar_gear_opens_settings_with_theme_and_language() {
    let mut h = harness(1100.0, new_app());
    h.get_by_label("Settings").click();
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.id.as_str()), Some("preferences"));
    assert!(h.query_by_label("Theme:").is_some() && h.query_by_label("Language:").is_some());
}

/// The magnifier opens the command palette; Enter runs the first command and closes it.
#[test]
fn title_bar_magnifier_opens_the_palette_and_runs_a_command() {
    let mut h = harness(1100.0, new_app());
    h.query_by_label_contains("Search commands").expect("the magnifier").click();
    h.run_steps(3);
    assert!(h.state().palette.is_some(), "the magnifier opens the palette");
    h.event(egui::Event::Text("check for updates".into()));
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().palette.is_none(), "running a command closes the palette");
}

/// Visual QA: PNGs of the title bar at the minimum window width and at 1100 px, in
/// `target/qa-casca/`. `cargo test -p localslides-ui-egui --lib qa_casca -- --ignored --nocapture`.
#[test]
#[ignore = "visual QA: renders PNGs for review"]
fn qa_casca_renders_the_shell_for_review() {
    let _gpu = crate::gpu_lock::gpu_lock();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/qa-casca");
    std::fs::create_dir_all(&dir).unwrap();
    for width in [900.0_f32, 1100.0] {
        for (name, mode) in [("light", "light"), ("dark", "dark"), ("settings", "light"), ("search", "light")] {
            let mut app = new_app();
            app.run("view.theme", json!({"mode": mode})).unwrap();
            let mut h = harness(width, app);
            match name {
                "settings" => {
                    h.state_mut().run("app.preferences", json!({})).unwrap();
                }
                "search" => h.state_mut().palette = Some((String::new(), 0)),
                _ => {}
            }
            h.run_steps(5);
            let path = dir.join(format!("slides-{name}-{}.png", width as u32));
            h.render().expect("render").save(&path).expect("save");
            println!("{}", path.display());
        }
    }
}
