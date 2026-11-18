# Interface catalogs

Translations of Local Slides' English source text, one directory per locale, read by `craft-i18n`
(vendored in `libs/craft-libs/crates/i18n`) and embedded by `crates/ui-egui/build.rs`.

```
locales/<code>/localslides.tsv   # this app's strings
locales/<code>/common.tsv        # optional: strings shared with the rest of the suite
```

Format, one row per line, columns separated by a TAB: `context ⇥ English source ⇥ translation`.
The header comment of each catalog documents the details (`@id`, `@plural`, `@msg`, escapes,
placeholders, and the rule that a trailing "." must appear in both columns).

- A missing translation shows the English text; coverage grows one row at a time and never fails a
  build, a test or a release. `crates/ui-egui/src/i18n.rs` has a test that every embedded catalog
  parses (`I18n::check_all`).
- In code: `tl!("English text")` where the string is drawn; `i18n::tr_fmt("Slide {n}", &[("n", …)])`
  for placeholders. A string that is also state (tab ids, command ids, saved values) stays English
  in the state and is translated only where it is painted.
- A new language: add `locales/<code>/localslides.tsv` (with its `@id language.name` row) and one
  entry to `LANGUAGES` in `crates/ui-egui/src/i18n.rs`. The picker and the system-language match
  pick it up.
- Clean room: write every row from the plain meaning of the English text. Do not copy another
  product's localisation or string table.
