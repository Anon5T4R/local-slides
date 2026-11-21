//! Every dialog and window of the app fits the window's minimum width (`with_min_inner_size` in
//! `apps/localslides/src/main.rs`), with a deck open and filled; the title bar, the command palette
//! and the update card are measured too. The bounding boxes come from the accesskit tree, as
//! `every_dialog_fits_the_narrow_window` does in the Local Keys suite.

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use localslides_engine::Session;
use serde_json::json;

use crate::dialogs::Dialog;
use crate::{Services, SlideApp};

/// The app's minimum inner width (`apps/localslides/src/main.rs`).
const MIN_WIDTH: f32 = 900.0;

/// A deck with slides, a text box, a table, a picture, a comment and a section: the dialogs draw
/// their lists and previews instead of their empty states.
fn filled_session() -> Session {
    let mut s = Session::with_new();
    s.execute("slide.new", &json!({"layout": "titleAndContent", "title": "Fotossíntese", "body": "Clorofila converte luz\nÁgua e gás carbônico"}))
        .unwrap();
    s.execute("slide.new", &json!({"layout": "titleAndContent", "title": "Resultados do trimestre", "body": "Receita\nDespesa\nMargem"})).unwrap();
    s.execute("shape.insert", &json!({"preset": "rect", "rect": [80, 120, 700, 160], "text": "Plano do trimestre"})).unwrap();
    s.execute("insert.table", &json!({"rows": 2, "cols": 2, "rect": [80, 80, 600, 160]})).unwrap();
    s.execute("comment.new", &json!({"slide": 1, "text": "Revisar os números do mês."})).ok();
    s.execute("section.add", &json!({"name": "Introdução"})).ok();
    s
}

fn harness_at(session: Session, width: f32) -> egui_kittest::Harness<'static, SlideApp> {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(width, 640.0)).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SlideApp::new(session, Services::default()),
    );
    h.run_steps(4);
    h
}

/// Every accesskit node of `root`'s subtree with a box stays inside the window's width.
fn assert_subtree_inside(root: &egui_kittest::Node<'_>, what: &str, width: f32) {
    for node in root.children_recursive() {
        // A field's text runs past its own box (egui clips the drawing): the field is the node.
        if node.accesskit_node().role() == Role::TextRun {
            continue;
        }
        let Some(rect) = node.accesskit_node().bounding_box() else { continue };
        if rect.width() > 0.0 {
            assert!(
                rect.x0 >= -0.5 && rect.x1 <= f64::from(width) + 0.5,
                "{what}: {:?} outside the {width} pt window: {rect:?}",
                node.accesskit_node().role()
            );
        }
    }
}

/// The windows on screen (a dialog, the palette, the update card) fit the window width.
fn assert_windows_inside(harness: &egui_kittest::Harness<'_, SlideApp>, what: &str, width: f32) -> usize {
    let mut seen = 0;
    for node in harness.query_all_by_role(Role::Window) {
        if node.accesskit_node().bounding_box().is_none() {
            continue;
        }
        seen += 1;
        assert_subtree_inside(&node, what, width);
    }
    seen
}

/// The dialogs that draw their own window (the pickers and the native show window are not dialogs
/// of the app's chrome).
const DIALOGS: &[&str] = &[
    "table",
    "slideSize",
    "sizePosition",
    "headerFooter",
    "hyperlink",
    "setupShow",
    "zoom",
    "comment",
    "altText",
    "outline",
    "renameSection",
    "reuseSlides",
    "renameShape",
    "renameLayout",
    "paragraph",
    "tabs",
    "font",
    "about",
    "preferences",
    "autocorrect",
    "gridOptions",
    "columns",
    "export",
    "symbol",
    "photoAlbum",
    "replaceFonts",
    "equation",
    "dateTime",
    "spelling",
    "pasteSpecial",
    "find",
    "accessibility",
    "language",
    "customShow",
    "chartData",
    "trim",
    "smartart",
    "bullets",
    "action",
    "themeColors",
    "themeFonts",
];

/// Every dialog fits the window's minimum width, with the deck filled.
#[test]
fn every_dialog_fits_the_narrow_window() {
    for id in DIALOGS {
        let mut h = harness_at(filled_session(), MIN_WIDTH);
        h.state_mut().dialog = Some(Dialog::new(id));
        h.run_steps(4);
        assert_eq!(h.state().dialog.as_ref().map(|d| d.id.as_str()), Some(*id), "{id} did not stay open");
        assert!(assert_windows_inside(&h, id, MIN_WIDTH) >= 1, "{id} draws no window");
    }
}

/// The title bar (document name and the suite's three buttons) fits the minimum width.
#[test]
fn the_title_bar_fits_the_narrow_window() {
    let h = harness_at(filled_session(), MIN_WIDTH);
    for node in h.root().children_recursive() {
        let Some(rect) = node.accesskit_node().bounding_box() else { continue };
        if rect.y1 > 38.0 || node.accesskit_node().role() == Role::TextRun {
            continue;
        }
        assert!(
            rect.x0 >= -0.5 && rect.x1 <= f64::from(MIN_WIDTH) + 0.5,
            "title bar: {:?} outside the window: {rect:?}",
            node.accesskit_node().role()
        );
    }
    for needle in ["Search commands", "Theme:", "Settings"] {
        let node = h.query_by_label_contains(needle).unwrap_or_else(|| panic!("{needle} is in the title bar"));
        let rect = node.rect();
        assert!(rect.max.x <= MIN_WIDTH && rect.min.x >= 0.0, "{needle} is outside the window: {rect:?}");
    }
}

/// The ribbon's tab row never runs past the minimum width: the tabs that don't fit go to the "»"
/// menu (the deck has a table, so a contextual tab is in the row too).
#[test]
fn the_ribbon_tabs_fit_the_narrow_window() {
    let h = harness_at(filled_session(), MIN_WIDTH);
    let mut tabs = 0;
    for node in h.query_all_by_role(Role::Button) {
        let rect = node.rect();
        if rect.min.y < 40.0 || rect.max.y > 76.0 {
            continue;
        }
        tabs += 1;
        assert!(rect.max.x <= MIN_WIDTH, "{:?} at {rect:?} runs past the window", node.accesskit_node().label());
    }
    assert!(tabs >= 6, "the tab row has its tabs: {tabs}");
    // Everything that does not fit is reachable from the "»" menu, which is inside the window.
    let more = h.get_by_label("More tabs").rect();
    assert!(more.max.x <= MIN_WIDTH && more.min.x >= 0.0, "the tab menu button is inside the window: {more:?}");
}

/// The command palette (`menus::palette`) fits the minimum width.
#[test]
fn the_palette_fits_the_narrow_window() {
    let mut h = harness_at(filled_session(), MIN_WIDTH);
    h.state_mut().palette = Some((String::new(), 0));
    h.run_steps(3);
    assert!(h.state().palette.is_some(), "the palette is shown");
    assert!(assert_windows_inside(&h, "palette", MIN_WIDTH) >= 1, "the palette draws no window");
}

/// The update card (`craft-update`, an offered release) fits the minimum width.
#[test]
fn the_update_card_fits_the_narrow_window() {
    let mut h = harness_at(filled_session(), MIN_WIDTH);
    let release = craft_update::Release { tag: "v9.9.9".into(), version: craft_update::version::parse("9.9.9"), ..craft_update::Release::default() };
    crate::dialogs::updates_ui::offer_for_test(h.state_mut(), release);
    h.run_steps(4);
    h.query_by_label("LATER").expect("the offered card");
    // The card is an `Area` (no `Window` role): every node below the ribbon must fit the width.
    for node in h.root().children_recursive() {
        let Some(rect) = node.accesskit_node().bounding_box() else { continue };
        if rect.y0 < 200.0 || node.accesskit_node().role() == Role::TextRun {
            continue;
        }
        if rect.width() > 0.0 {
            assert!(
                rect.x0 >= -0.5 && rect.x1 <= f64::from(MIN_WIDTH) + 0.5,
                "update card: {:?} outside the window: {rect:?}",
                node.accesskit_node().role()
            );
        }
    }
}
