//! The three deck actions: request -> messages, answer -> [`Outcome`].
//!
//! The window applies an outcome through the engine's own commands inside one undo group
//! ([`localslides_engine::Session::grouped`]), so the AI never writes the `.pptx` directly and a
//! failed apply rolls back. The model's answer is read by the shared tolerant extractor
//! ([`crate::extract_json`]); the structure is checked here, so a bad answer is a message, not a
//! half-built deck.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use local_ai::Msg;

use crate::extract_json;
use crate::prompts::{fill, for_lang};

/// One slide's text, as the window reads it (`slide.inspect`) and as the model gets it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SlideText {
    /// Zero-based position in the deck, so the answer can name the slide it changed.
    pub index: usize,
    pub title: String,
    /// The bullets, one per line (the body placeholder's text).
    pub body: String,
}

/// A slide's place in the deck: the model labels it, the app picks the layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Role {
    /// The deck's opening slide (title, no bullets).
    Title,
    /// A divider before a part of the deck.
    Section,
    /// A regular slide with bullets.
    #[default]
    Content,
}

impl Role {
    /// The label, as the contract writes it.
    pub fn id(self) -> &'static str {
        match self {
            Role::Title => "title",
            Role::Section => "section",
            Role::Content => "content",
        }
    }

    /// The role `name` labels; anything else is a content slide (a closed vocabulary).
    pub fn from_id(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "title" => Role::Title,
            "section" => Role::Section,
            _ => Role::Content,
        }
    }

    /// The layout kind the engine's `slide.new` takes for this role.
    pub fn layout(self) -> &'static str {
        match self {
            Role::Title => "title",
            Role::Section => "sectionHeader",
            Role::Content => "titleAndContent",
        }
    }
}

/// One slide the model planned for a new deck.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedSlide {
    pub title: String,
    /// One bullet per line when applied; a leading tab nests it under the bullet before.
    pub bullets: Vec<String>,
    pub role: Role,
}

/// What the user asked for.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// A free description -> a whole deck of `slides` slides.
    GenerateDeck { topic: String, slides: u32 },
    /// One slide's text plus an instruction -> that slide's new title and body.
    EditSlide { index: usize, title: String, body: String, instruction: String },
    /// Several slides -> the same slides, corrected.
    FixSlides { slides: Vec<SlideText> },
}

/// What the answer became, ready to apply with engine commands.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Slides to append, in order.
    Deck { slides: Vec<PlannedSlide> },
    /// New text for the slide that was asked about (either part may be absent).
    Slide { title: Option<String>, body: Option<String> },
    /// The slides that changed, by index.
    Corrected { slides: Vec<SlideText> },
}

impl Request {
    /// The short label the status strip shows while the worker runs.
    pub fn label(&self) -> &'static str {
        match self {
            Request::GenerateDeck { .. } => "Generating the presentation",
            Request::EditSlide { .. } => "Rewriting the slide",
            Request::FixSlides { .. } => "Correcting the slides",
        }
    }

    /// The subject appended to the label (the topic, the slide, how many slides).
    pub fn subject(&self) -> String {
        match self {
            Request::GenerateDeck { topic, slides } => format!("{topic} ({slides} slides)"),
            Request::EditSlide { index, .. } => format!("slide {}", index + 1),
            Request::FixSlides { slides } => match slides.as_slice() {
                [one] => format!("slide {}", one.index + 1),
                many => format!("{} slides", many.len()),
            },
        }
    }

    /// The slide the undo step names.
    pub fn undo_label(&self) -> &'static str {
        match self {
            Request::GenerateDeck { .. } => "AI: new presentation",
            Request::EditSlide { .. } => "AI: rewrite slide",
            Request::FixSlides { .. } => "AI: correct slides",
        }
    }
}

/// How many slides one generated deck may have, whatever the dialog asked for.
pub const MAX_SLIDES: u32 = 40;

/// The conversation one request sends: a system turn and one user turn.
pub fn messages(request: &Request, lang: &str) -> Result<Vec<Msg>, String> {
    let prompts = for_lang(lang);
    let system = Msg::System(fill(prompts.system, &[("answer_in", prompts.answer_in)]));
    let user = match request {
        Request::GenerateDeck { topic, slides } => {
            let topic = topic.trim();
            if topic.is_empty() {
                return Err("the description is empty".into());
            }
            let count = (*slides).clamp(1, MAX_SLIDES);
            fill(prompts.generate, &[("topic", topic), ("slides", &count.to_string()), ("answer_in", prompts.answer_in)])
        }
        Request::EditSlide { index, title, body, instruction } => {
            let instruction = instruction.trim();
            if instruction.is_empty() {
                return Err("the instruction is empty".into());
            }
            let slide = serde_json::to_string(&SlideText { index: *index, title: title.clone(), body: body.clone() })
                .map_err(|e| format!("the slide could not be written as JSON: {e}"))?;
            fill(prompts.edit, &[("instruction", instruction), ("slide", &slide), ("answer_in", prompts.answer_in)])
        }
        Request::FixSlides { slides } => {
            if slides.is_empty() {
                return Err("there are no slides to correct".into());
            }
            let text = serde_json::to_string(slides).map_err(|e| format!("the slides could not be written as JSON: {e}"))?;
            fill(prompts.fix, &[("slides", &text), ("answer_in", prompts.answer_in)])
        }
    };
    Ok(vec![system, Msg::User(user)])
}

/// Turns one answer into what the window applies. Structural problems are errors that name the
/// slide that failed; the requested count is a target, so a model that answers a slide more or
/// fewer is accepted (the window applies what came).
pub fn interpret(request: &Request, answer: &str) -> Result<Outcome, String> {
    match request {
        Request::GenerateDeck { slides, .. } => {
            let answer: DeckAnswer = extract_json(answer)?;
            if answer.slides.is_empty() {
                return Err("the model did not answer any slide".into());
            }
            let mut planned = Vec::with_capacity(answer.slides.len());
            for (index, slide) in answer.slides.into_iter().enumerate() {
                let title = slide.title.trim().to_string();
                if title.is_empty() {
                    return Err(format!("the model answered slide {} without a title", index + 1));
                }
                let bullets =
                    lines(slide.bullets.or(slide.body)).into_iter().map(|line| clean_line(&line)).filter(|line| !line.trim().is_empty()).collect();
                // A model that omits the role still gets a usable deck: the first slide opens it.
                let role = match slide.role.as_deref() {
                    Some(role) => Role::from_id(role),
                    None if index == 0 => Role::Title,
                    None => Role::Content,
                };
                planned.push(PlannedSlide { title, bullets, role });
            }
            planned.truncate((*slides).clamp(1, MAX_SLIDES) as usize);
            Ok(Outcome::Deck { slides: planned })
        }
        Request::EditSlide { .. } => {
            let answer: SlideAnswer = extract_json(answer)?;
            let title = answer.title.map(|title| title.trim().to_string()).filter(|title| !title.is_empty());
            let body = answer.body.or(answer.bullets).map(joined).filter(|body| !body.trim().is_empty());
            if title.is_none() && body.is_none() {
                return Err("the model did not answer a title or a body".into());
            }
            Ok(Outcome::Slide { title, body })
        }
        Request::FixSlides { slides } => {
            let answer: FixAnswer = extract_json(answer)?;
            let mut corrected = Vec::with_capacity(answer.slides.len());
            for slide in answer.slides {
                if slide.index >= slides.len() {
                    return Err(format!("the model answered slide {}, there are {}", slide.index + 1, slides.len()));
                }
                let title = slide.title.map(|title| title.trim().to_string()).filter(|title| !title.is_empty());
                let body = slide.body.or(slide.bullets).map(joined).filter(|body| !body.trim().is_empty());
                if title.is_none() && body.is_none() {
                    continue;
                }
                corrected.push(SlideText { index: slide.index, title: title.unwrap_or_default(), body: body.unwrap_or_default() });
            }
            if corrected.is_empty() {
                return Err("the model did not correct any slide".into());
            }
            Ok(Outcome::Corrected { slides: corrected })
        }
    }
}

/// The bullets of one box, from either shape a model answers (one string with lines, or a list).
#[derive(Deserialize)]
#[serde(untagged)]
enum Lines {
    One(String),
    Many(Vec<String>),
}

/// The lines of `lines`, in order (an empty list for a missing value).
fn lines(lines: Option<Lines>) -> Vec<String> {
    match lines {
        None => Vec::new(),
        Some(Lines::One(text)) => text.lines().map(str::to_string).collect(),
        Some(Lines::Many(lines)) => lines,
    }
}

/// The text `text.set` writes for a body: one bullet per line.
fn joined(lines: Lines) -> String {
    match lines {
        Lines::One(text) => text,
        Lines::Many(lines) => lines.iter().map(|line| clean_line(line)).filter(|line| !line.is_empty()).collect::<Vec<_>>().join("\n"),
    }
}

/// A bullet as the engine should show it: trimmed, without the marker a model sometimes adds
/// (`- `, `* `, `• `). **Leadings tabs stay**: the engine reads them as the outline level, which is
/// how a sub-bullet nests under the one before it.
fn clean_line(line: &str) -> String {
    let line = line.trim_end();
    let tabs: String = line.chars().take_while(|c| *c == '\t').collect();
    let rest = line[tabs.len()..].trim_start();
    for marker in ["- ", "* ", "• "] {
        if let Some(after) = rest.strip_prefix(marker) {
            return format!("{tabs}{}", after.trim_start());
        }
    }
    format!("{tabs}{rest}")
}

#[derive(Deserialize)]
struct DeckAnswer {
    #[serde(default)]
    slides: Vec<DeckSlide>,
}

#[derive(Deserialize)]
struct DeckSlide {
    #[serde(default)]
    title: String,
    /// `title`, `section` or `content` (a leading tab nests a bullet under the one before it).
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    bullets: Option<Lines>,
    /// A model that answers the body as one text instead of a bullet list still fits.
    #[serde(default)]
    body: Option<Lines>,
}

#[derive(Deserialize)]
struct SlideAnswer {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    body: Option<Lines>,
    #[serde(default)]
    bullets: Option<Lines>,
}

#[derive(Deserialize)]
struct FixAnswer {
    #[serde(default)]
    slides: Vec<FixSlide>,
}

#[derive(Deserialize)]
struct FixSlide {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    body: Option<Lines>,
    #[serde(default)]
    bullets: Option<Lines>,
}

/// The value a caller logs or shows for an outcome (test helper and diagnostics).
pub fn outcome_json(outcome: &Outcome) -> Value {
    match outcome {
        Outcome::Deck { slides } => serde_json::json!({
            "slides": slides.iter().map(|s| serde_json::json!({"title": s.title, "bullets": s.bullets})).collect::<Vec<_>>()
        }),
        Outcome::Slide { title, body } => serde_json::json!({"title": title, "body": body}),
        Outcome::Corrected { slides } => serde_json::json!({"slides": slides}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deck() -> Request {
        Request::GenerateDeck { topic: "fotossíntese".into(), slides: 3 }
    }

    #[test]
    fn a_deck_answer_becomes_slides_in_order() {
        let answer = r#"{"slides":[{"title":"Fotossíntese","bullets":[]},{"title":"Luz","bullets":["clorofila","- água"]}]}"#;
        match interpret(&deck(), answer).expect("a deck") {
            Outcome::Deck { slides } => {
                assert_eq!(slides.len(), 2);
                assert_eq!(slides[0].title, "Fotossíntese");
                assert!(slides[0].bullets.is_empty());
                assert_eq!(slides[1].bullets, ["clorofila", "água"], "the marker a model adds is dropped");
            }
            other => panic!("expected a deck, got {other:?}"),
        }
    }

    /// A reasoning block and a markdown fence are the extractor's problem, not this action's.
    #[test]
    fn the_extractor_reads_through_reasoning_and_fences() {
        let answer = "<think>{not the answer}</think>\n```json\n{\"slides\":[{\"title\":\"Um\",\"bullets\":[\"x\"]}]}\n```";
        assert!(matches!(interpret(&deck(), answer), Ok(Outcome::Deck { .. })));
    }

    /// The role vocabulary: the model labels the slide, the app picks the layout. A model that
    /// omits it still gets a usable deck (the first slide opens it).
    #[test]
    fn slide_roles_come_from_the_label_and_the_first_defaults_to_title() {
        let answer = r#"{"slides":[{"title":"Abertura"},{"role":"section","title":"Parte"},{"role":"content","title":"Um","bullets":["x"]}]}"#;
        match interpret(&deck(), answer).expect("deck") {
            Outcome::Deck { slides } => {
                assert_eq!(slides.iter().map(|slide| slide.role).collect::<Vec<_>>(), [Role::Title, Role::Section, Role::Content]);
                assert_eq!(slides[1].role.layout(), "sectionHeader");
                assert_eq!(slides[2].role.layout(), "titleAndContent");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_invented_role_is_a_content_slide() {
        assert_eq!(Role::from_id("quote"), Role::Content);
        let answer = r#"{"slides":[{"role":"title","title":"A"},{"role":"quote","title":"B"}]}"#;
        match interpret(&deck(), answer).expect("deck") {
            Outcome::Deck { slides } => assert_eq!(slides[1].role, Role::Content),
            other => panic!("{other:?}"),
        }
    }

    /// A leading tab is the outline level the engine reads, so a sub-bullet keeps it.
    #[test]
    fn a_nested_bullet_keeps_its_tab() {
        let answer = r#"{"slides":[{"role":"title","title":"A"},{"role":"content","title":"B","bullets":["um","\tdois","\t\ttrês"]}]}"#;
        match interpret(&deck(), answer).expect("deck") {
            Outcome::Deck { slides } => assert_eq!(slides[1].bullets, ["um", "\tdois", "\t\ttrês"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_slide_without_a_title_is_refused_with_its_number() {
        let error = interpret(&deck(), r#"{"slides":[{"title":"Um"},{"bullets":["x"]}]}"#).expect_err("no title");
        assert!(error.contains("slide 2"), "{error}");
    }

    #[test]
    fn the_requested_count_caps_the_deck() {
        let answer = r#"{"slides":[{"title":"Um"},{"title":"Dois"},{"title":"Três"}]}"#;
        let request = Request::GenerateDeck { topic: "x".into(), slides: 2 };
        match interpret(&request, answer).expect("deck") {
            Outcome::Deck { slides } => assert_eq!(slides.len(), 2),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_body_answer_as_one_text_is_split_into_bullets() {
        let answer = r#"{"slides":[{"title":"Um","body":"um\ndois"}]}"#;
        match interpret(&deck(), answer).expect("deck") {
            Outcome::Deck { slides } => assert_eq!(slides[0].bullets, ["um", "dois"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_edit_answers_the_new_title_and_body() {
        let request = Request::EditSlide { index: 1, title: "Luz".into(), body: "clorofila".into(), instruction: "encurta".into() };
        match interpret(&request, r#"{"title":"Luz","body":"clorofila e água"}"#).expect("slide") {
            Outcome::Slide { title, body } => {
                assert_eq!(title.as_deref(), Some("Luz"));
                assert_eq!(body.as_deref(), Some("clorofila e água"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_edit_with_nothing_to_change_is_refused() {
        let request = Request::EditSlide { index: 0, title: "A".into(), body: String::new(), instruction: "x".into() };
        assert!(interpret(&request, r#"{"title":"  "}"#).is_err());
    }

    #[test]
    fn a_correction_names_its_slides_by_index() {
        let request = Request::FixSlides {
            slides: vec![
                SlideText { index: 0, title: "Intrudução".into(), body: "Aguas".into() },
                SlideText { index: 1, title: "Luz".into(), body: "Clorofila".into() },
            ],
        };
        let answer = r#"{"slides":[{"index":0,"title":"Introdução","body":"Águas"}]}"#;
        match interpret(&request, answer).expect("corrections") {
            Outcome::Corrected { slides } => {
                assert_eq!(slides.len(), 1);
                assert_eq!(slides[0].index, 0);
                assert_eq!(slides[0].title, "Introdução");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_correction_for_a_slide_that_is_not_there_is_refused() {
        let request = Request::FixSlides { slides: vec![SlideText { index: 0, title: "A".into(), body: String::new() }] };
        let error = interpret(&request, r#"{"slides":[{"index":3,"title":"B"}]}"#).expect_err("out of range");
        assert!(error.contains("slide 4"), "{error}");
    }

    #[test]
    fn an_empty_description_is_refused_before_the_request() {
        assert!(messages(&Request::GenerateDeck { topic: "   ".into(), slides: 3 }, "en").is_err());
    }

    #[test]
    fn an_empty_instruction_is_refused_before_the_request() {
        let request = Request::EditSlide { index: 0, title: "A".into(), body: String::new(), instruction: "  ".into() };
        assert!(messages(&request, "en").is_err());
    }

    #[test]
    fn the_messages_carry_the_contract_in_the_language() {
        let messages = messages(&deck(), "pt-br").expect("messages");
        assert_eq!(messages.len(), 2);
        let user = match &messages[1] {
            Msg::User(text) => text,
            other => panic!("{other:?}"),
        };
        assert!(user.contains("\"slides\"") && user.contains("fotossíntese"));
        assert!(user.contains("português"), "answer in the user's language: {user}");
    }

    #[test]
    fn the_slide_context_travels_as_json() {
        let request = Request::EditSlide { index: 2, title: "Luz".into(), body: "clorofila".into(), instruction: "traduz".into() };
        let messages = messages(&request, "en").expect("messages");
        let Msg::User(text) = &messages[1] else { panic!("user turn") };
        assert!(text.contains(r#"{"index":2,"title":"Luz","body":"clorofila"}"#), "{text}");
    }
}
