//! `somadoc-core`: core of the SomaDoc document engine.
//!
//! Provides IR (unified intermediate representation) → Markdown serialization,
//! plain-text extraction, and the shared error type.

pub mod error;
pub mod extract;
pub mod ir;
pub mod to_markdown;

pub use error::CoreError;
pub use extract::extract_text;
pub use ir::{BlockNode, InlineNode};
pub use to_markdown::ir_to_markdown;
