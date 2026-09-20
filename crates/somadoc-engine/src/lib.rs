//! `somadoc-engine` — a pure-Rust engine that turns Markdown (plus handwritten
//! LaTeX) into publish-grade Typst documents, as PDF or per-page SVG.
//!
//! The engine compiles the Typst source **in-process** (via the [`typst`] crate),
//! so it needs no `pandoc`, no LaTeX distribution, and no external sandbox.
//!
//! ```
//! use somadoc_engine::{RenderEngine, RenderRequest, TypstEngine};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Templates are resolved relative to the directory passed to `new`; this
//! // crate ships a reference template under `<crate>/templates`.
//! let templates_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/templates");
//! let engine = TypstEngine::new(templates_dir);
//!
//! let req = RenderRequest {
//!     template_id: Some("soma".into()),
//!     content: Some("# Hello\n\nA paragraph.".into()),
//!     ..Default::default()
//! };
//!
//! let output = engine.typeset(&req).await?;
//! assert!(output.bytes.starts_with(b"%PDF"));
//! # Ok(())
//! # }
//! ```
//!
//! An empty `template_id` renders with the built-in default style instead of a
//! `template.typ`.

pub mod common;
pub mod images;
pub mod latex_commands;
pub mod svg_out;
pub mod types;
pub mod typst;

pub use types::{
    InlineImage, RenderEngine, RenderError, RenderOutput, RenderRequest, RenderResult,
};
pub use typst::TypstEngine;
