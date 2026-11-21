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

/// Like the suite's `craft-palette`: a click outside the palette closes it, a click inside does
/// not. The magnifier's own opening click is outside, but it belongs to the opening frame.
#[test]
fn palette_closes_on_click_outside_but_not_inside() {
    fn click(h: &mut Harness<'static, SlideApp>, pos: egui::Pos2) {
        h.input_mut().events.extend([
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        ]);
        h.run_steps(1);
        h.input_mut().events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(2);
    }

    let mut h = harness(1100.0, new_app());
    // The palette window is fixed 520×380, anchored CENTER_TOP with a 90 px offset: in a
    // 1100×560 harness it spans (290, 90)–(810, 470).
    h.state_mut().palette = Some((String::new(), 0));
    h.run_steps(3);
    assert!(h.state().palette.is_some(), "the palette is shown");

    // Inside, on the window's left edge (no widget there): stays open.
    click(&mut h, egui::pos2(293.0, 300.0));
    assert!(h.state().palette.is_some(), "a click inside does not close the palette");

    // Outside, at the top-left corner of the window: closes.
    click(&mut h, egui::pos2(4.0, 4.0));
    assert!(h.state().palette.is_none(), "a click outside closes the palette");
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
