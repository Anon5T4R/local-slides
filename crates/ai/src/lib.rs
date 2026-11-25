//! Remote AI for Local Slides: what the window asks the model, and what it does with the answer.
//!
//! The model is remote by design (the suite is pure Rust; there is no local engine). What the user
//! configures is a connection from the shared `local-ai` crate: a provider preset, its base URL, a
//! model and their own key in the OS keychain.
//!
//! Three pieces, each testable without a network:
//!
//! - [`prompts`]: the fragments, localized, with the JSON contract written out (the contract's keys
//!   are never translated).
//! - [`actions`]: the three deck actions, turning a request plus the slide context into messages
//!   and the answer back into an [`Outcome`] the window applies through the engine's own commands,
//!   inside one undo group.
//! - [`backend`]: the seam to the network ([`LiveBackend`] over `local-ai`; the tests use a mock).
//!
//! The tolerant JSON extractor lives in the shared source (`local_ai::extract`) so every app answers
//! the same way to reasoning blocks, fences and prose, and there is one implementation to fix.

pub mod actions;
pub mod backend;
pub mod prompts;

pub use actions::{Outcome, PlannedSlide, Request, Role, SlideText};
pub use backend::{Backend, LiveBackend};
/// The shared client, for the connections panel (presets, keychain and the model list).
pub use local_ai;
/// The extractor the actions use, re-exported so a caller does not need `local-ai` directly.
pub use local_ai::extract::extract_json;

/// The identity the connections file and the keychain entries belong to.
pub const AI_APP_ID: &str = "LocalSlides";
/// The project URL the OpenRouter headers advertise.
pub const AI_APP_URL: &str = "https://github.com/Styx-Studios/Local-Slides";
