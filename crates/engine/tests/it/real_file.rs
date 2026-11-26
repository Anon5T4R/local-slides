//! A deck written by another application: opened, saved as `.deckcraft` to a temporary file,
//! reopened from disk and exported again. Set `LOCALSLIDES_PPTX_IN=<file>` to point it at one;
//! the corpus lives outside the repository, so CI skips it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

fn temp_file(extension: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("localslides-real-file-{}-{n}.{extension}", std::process::id()))
}

#[test]
fn real_file_opens_saves_to_a_temporary_deck_and_reopens() {
    let Ok(path) = std::env::var("LOCALSLIDES_PPTX_IN") else { return };
    let pptx = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let opened = localslides_pptx::import(&pptx).expect("import the real file");
    assert!(!opened.slides.is_empty(), "the real file imported no slides");
    assert!(opened.validate().is_empty(), "{:?}", opened.validate());

    let deck = temp_file("deckcraft");
    std::fs::write(&deck, localslides_format::save(&opened).expect("save deck")).unwrap_or_else(|e| panic!("write {}: {e}", deck.display()));
    let bytes = std::fs::read(&deck).unwrap_or_else(|e| panic!("read {}: {e}", deck.display()));
    std::fs::remove_file(&deck).expect("remove the temporary deck");

    let reopened = localslides_format::load(&bytes).expect("load the saved deck");
    assert_eq!(reopened, opened, "the saved deck must hold everything the opened deck had");
    assert!(reopened.validate().is_empty(), "{:?}", reopened.validate());

    let again = localslides_pptx::export(&reopened).expect("export the reopened deck");
    let reimported = localslides_pptx::import(&again).expect("reimport the exported deck");
    assert_eq!(reimported, opened, "the exported deck must reimport as the file it came from");
}
