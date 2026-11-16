//! The seam to the network.
//!
//! [`LiveBackend`] is the suite's client (`local-ai`): the user's own connection and key, the
//! OpenAI-compatible wire format, streaming over SSE, and the provider's model list. The window
//! never talks HTTP itself; the tests answer from a canned string through the same [`Backend`]
//! trait, so no test opens a socket.

use local_ai::{Connection, Msg};

/// What the AI layer needs from a model: one streamed answer, and the provider's model list.
pub trait Backend: Send + Sync {
    /// Sends `history` and streams the answer: `on_delta` receives each piece as it arrives, and the
    /// whole text comes back.
    fn stream(&self, connection: &Connection, history: &[Msg], on_delta: &mut dyn FnMut(&str)) -> Result<String, String>;

    /// The provider's models, for the picker.
    fn models(&self, connection: &Connection) -> Result<Vec<String>, String>;
}

/// The suite's client: a per-app identity (its config folder and keychain service) over `local-ai`.
pub struct LiveBackend {
    app: local_ai::App,
}

impl LiveBackend {
    /// `id` is the folder under the user's data dir and the keychain service, `url` the project the
    /// OpenRouter headers advertise.
    pub fn new(id: &'static str, url: &'static str) -> Self {
        Self { app: local_ai::App { id, url } }
    }

    /// The identity the saved connections belong to.
    pub fn app(&self) -> &local_ai::App {
        &self.app
    }
}

impl Backend for LiveBackend {
    fn stream(&self, connection: &Connection, history: &[Msg], on_delta: &mut dyn FnMut(&str)) -> Result<String, String> {
        local_ai::chat_stream(&self.app, connection, history, on_delta)
    }

    fn models(&self, connection: &Connection) -> Result<Vec<String>, String> {
        local_ai::list_models(&self.app, connection)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backend that answers from a canned string, one piece at a time: what the UI tests use.
    pub struct MockBackend {
        pub answer: String,
        pub fail_with: Option<String>,
    }

    impl MockBackend {
        pub fn new(answer: &str) -> Self {
            Self { answer: answer.to_string(), fail_with: None }
        }

        pub fn failing(reason: &str) -> Self {
            Self { answer: String::new(), fail_with: Some(reason.to_string()) }
        }
    }

    impl Backend for MockBackend {
        fn stream(&self, _connection: &Connection, _history: &[Msg], on_delta: &mut dyn FnMut(&str)) -> Result<String, String> {
            if let Some(reason) = &self.fail_with {
                return Err(reason.clone());
            }
            // Two deltas, so the UI path that accumulates them is exercised.
            let (head, tail) = self.answer.split_at(self.answer.len() / 2);
            on_delta(head);
            on_delta(tail);
            Ok(self.answer.clone())
        }

        fn models(&self, _connection: &Connection) -> Result<Vec<String>, String> {
            Ok(vec!["mock-model".to_string()])
        }
    }

    fn connection() -> Connection {
        Connection {
            id: "c1".into(),
            provider: "custom".into(),
            label: "Mock".into(),
            base_url: "http://localhost:1/v1".into(),
            model: "mock-model".into(),
        }
    }

    #[test]
    fn the_mock_streams_the_whole_answer_in_pieces() {
        let backend = MockBackend::new("resposta completa");
        let mut pieces = String::new();
        let answer = backend.stream(&connection(), &[Msg::User("oi".into())], &mut |delta| pieces.push_str(delta)).expect("stream");
        assert_eq!(answer, "resposta completa");
        assert_eq!(pieces, answer);
    }

    #[test]
    fn a_failing_backend_reports_the_reason() {
        let backend = MockBackend::failing("sem chave");
        assert_eq!(backend.stream(&connection(), &[], &mut |_| {}).unwrap_err(), "sem chave");
    }

    /// The live backend exists and carries the app identity (no request is made here).
    #[test]
    fn the_live_backend_carries_the_app_identity() {
        let backend = LiveBackend::new(crate::AI_APP_ID, crate::AI_APP_URL);
        assert_eq!(backend.app().id, "LocalSlides");
        assert_eq!(backend.app().url, "https://github.com/Anon5T4R/local-slides");
    }
}
