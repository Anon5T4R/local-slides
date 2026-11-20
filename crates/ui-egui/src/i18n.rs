//! Interface languages, on `craft-i18n` (vendored in `libs/craft-libs/crates/i18n`).
//!
//! Strings in the code stay English and are the lookup keys; the catalogs under
//! `i18n/locales/<code>/localslides.tsv` map them to display text when the UI is drawn. A string
//! without a translation shows in English, so coverage grows one row at a time and a missing
//! translation never fails a build, a test or a release. The catalogs are embedded by
//! `crates/ui-egui/build.rs` (`CRAFT_I18N_DIR` overrides the directory).
//!
//! Translate at display time only: a string that is also state (a ribbon tab id, a command id, a
//! saved value) stays English in that state and goes through [`tr`] where it is drawn.
//!
//! Adding a language: drop `i18n/locales/<code>/localslides.tsv` in (with its `@id language.name`
//! row) and add one row to [`LANGUAGES`].

use std::cell::Cell;
use std::sync::LazyLock;

use craft_i18n::I18n;

include!(concat!(env!("OUT_DIR"), "/craft_i18n.rs"));

/// The `language` setting that follows the operating system's language.
pub const AUTO: &str = "auto";

/// The languages this app offers (BCP 47, lowercase), when their catalogs are embedded. English
/// first: it is the fallback and the source language. `pt-br` is Brazilian Portuguese (`pt-BR` and
/// `pt-BR-*` resolve to it); `es` is Spanish, neutral across Spain and Latin America (`es-ES`,
/// `es-MX` and `es-419` resolve to it).
pub const LANGUAGES: [&str; 3] = ["en", "pt-br", "es"];

/// The embedded catalogs, offering exactly [`LANGUAGES`]. Built on first use; a locale is parsed
/// on its first lookup.
static I18N: LazyLock<I18n> = LazyLock::new(|| {
    let (i18n, problems) = I18n::new(CRAFT_I18N, &LANGUAGES);
    problems.iter().for_each(|p| log::warn!("interface catalogs: {p}"));
    i18n
});

thread_local! {
    /// The language of the current frame. The UI runs on one thread, so a thread-local keeps every
    /// widget's lookup free of arguments.
    static CURRENT: Cell<craft_i18n::Lang> = const { Cell::new(craft_i18n::Lang::EN) };
}

/// The language `setting` asks for: `auto` (the system's language, English when there is no
/// catalog for it) or a code from [`LANGUAGES`]. Unit tests always resolve `auto` to English, so
/// the UI tests read the same labels on every machine.
pub fn resolve(setting: &str) -> craft_i18n::Lang {
    if setting == AUTO {
        return if cfg!(test) { craft_i18n::Lang::EN } else { I18N.system_lang() };
    }
    I18N.match_tag(setting).or_else(|| I18N.from_code(setting)).unwrap_or(craft_i18n::Lang::EN)
}

/// Sets the language of the following frames, from the `language` setting.
pub fn set_language(setting: &str) {
    let lang = resolve(setting);
    CURRENT.with(|current| current.set(lang));
}

thread_local! {
    /// The `language` setting [`apply`] last resolved, so the per-frame call is a string compare.
    static APPLIED: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Applies the `language` setting when it changed since the last call. Cheap enough per frame.
pub fn apply(setting: &str) {
    let changed = APPLIED.with(|applied| {
        let mut applied = applied.borrow_mut();
        if applied.as_deref() == Some(setting) {
            return false;
        }
        *applied = Some(setting.to_string());
        true
    });
    if changed {
        set_language(setting);
    }
}

/// The language of this frame.
pub fn current() -> craft_i18n::Lang {
    CURRENT.with(Cell::get)
}

/// The code of the current language (`en`, `pt-br`).
pub fn code() -> &'static str {
    I18N.code(current())
}

/// `source` (English) in the current language, else in English.
pub fn tr(source: &str) -> &str {
    I18N.t(current(), source)
}

/// `source` (English) in the current language under a disambiguating `context` (the catalog's
/// first column), else the plain row, else English.
pub fn tr_ctx<'a>(context: &str, source: &'a str) -> &'a str {
    I18N.tc(current(), context, source)
}

/// `source` (English) in the current language, with `{name}` placeholders filled.
pub fn tr_fmt(source: &str, args: &[(&str, &str)]) -> String {
    craft_i18n::fmt(tr(source), args)
}

/// The offered languages as (code, the name each shows under, in its own language).
pub fn languages() -> Vec<(&'static str, &'static str)> {
    LANGUAGES.iter().map(|&code| (code, name_of(code))).collect()
}

/// A language's display name from its catalog's `@id language.name` row, else its code.
pub fn name_of(code: &str) -> &'static str {
    match I18N.from_code(code) {
        Some(lang) => I18N.name(lang),
        None => LANGUAGES.iter().copied().find(|c| *c == code).unwrap_or("English"),
    }
}

/// A string in the current language: `tl!("About")`.
#[macro_export]
macro_rules! tl {
    ($s:expr) => {
        $crate::i18n::tr($s)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_parses() {
        // A malformed row or locale is a warning at run time (the English text shows); the gate
        // here catches it before it ships.
        let problems = I18N.check_all();
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn english_is_the_source_and_the_fallback() {
        set_language("en");
        assert_eq!(tr("About"), "About");
        set_language("pt-br");
        assert_eq!(code(), "pt-br");
        assert_eq!(tr("a string no catalog has"), "a string no catalog has");
        set_language("es");
        assert_eq!(code(), "es");
        set_language(AUTO);
        assert_eq!(code(), "en", "tests resolve auto to English");
    }

    #[test]
    fn every_offered_language_has_a_catalog_and_a_name() {
        for code in LANGUAGES.iter().filter(|c| **c != "en") {
            assert!(I18N.from_code(code).is_some(), "{code} has no embedded catalog");
            assert_ne!(name_of(code), *code, "{code} has no `@id language.name` row");
        }
    }

    #[test]
    fn a_language_the_app_does_not_offer_resolves_to_english() {
        assert_eq!(resolve("tlh"), craft_i18n::Lang::EN);
        assert_eq!(resolve("pt-BR"), resolve("pt-br"));
    }
}
