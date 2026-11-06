//! A single-threaded Ladybird browser API, independent of any window system or
//! command transport. Drive [`Browser::poll`] from your own loop, consume its
//! events, and present [`Frame`] pixels wherever you choose.

/// Snapshot identity, or the explicit milestone tag supplied by release CI.
pub const VERSION: &str = env!("BUG_VERSION");
/// Full source commit; `unversioned` for a source tree without Git metadata.
pub const COMMIT: &str = env!("BUG_COMMIT");

mod browser;
pub mod clients;
mod engine;
mod evaluation;
pub mod protocol;
mod session;
pub use browser::*;
pub use engine::MouseInput;
pub use evaluation::{EvaluationId, EvaluationResult};
pub use session::{SavedView, SessionSnapshot, ViewMetadata};
