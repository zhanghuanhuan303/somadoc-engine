//! Common types for the rendering engine.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A render request.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct RenderRequest {
    /// Template slug. When set, it must be a directory name under the engine's
    /// template root containing a `template.typ` entry point. When absent, the
    /// engine's built-in default style is used.
    #[serde(default)]
    pub template_id: Option<String>,
    /// Full Markdown (including frontmatter); when present, `ir` is ignored.
    #[serde(default)]
    pub content: Option<String>,
    /// IR structure (used to generate Markdown when `content` is absent).
    #[serde(default)]
    pub ir: Option<serde_json::Value>,
    /// Frontend variables (merged into the Markdown frontmatter; keys already
    /// present in the content take precedence).
    #[serde(default)]
    pub variables: HashMap<String, String>,
    /// Whether to output a table of contents.
    #[serde(default)]
    pub toc: Option<String>,
    /// Live-preview session ID (optional). When set, `typeset_svg` reuses a
    /// persistent compilation context across calls so that unchanged parts of the
    /// document are not re-laid-out.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Inline images. Callers send the bytes base64-encoded and reference
    /// them from Markdown as `![alt](/somadoc-img/<id>)`; the engine registers
    /// each one under that virtual path so the compiled document can resolve it.
    #[serde(default)]
    pub images: Vec<InlineImage>,
}

/// An inline image: referenced in Markdown via `![alt](/somadoc-img/<id>)`; the
/// bytes and layout are carried by this struct.
#[derive(Debug, Clone, Deserialize)]
pub struct InlineImage {
    /// Virtual path id (without the `/somadoc-img/` prefix).
    pub id: String,
    /// Image bytes (base64-encoded).
    pub data: String,
    /// Layout: `center` (default) / `left` / `right` / `wrap-left` / `wrap-right`.
    #[serde(default)]
    pub align: Option<String>,
}

/// Render output.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderOutput {
    /// Result bytes: PDF for [`RenderEngine::typeset`], or a JSON envelope
    /// (`{"pages": [...], "anchors": [...]}`) for [`RenderEngine::typeset_svg`].
    pub bytes: Vec<u8>,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Engine id that produced this output (e.g. `"typst"`).
    #[serde(default)]
    pub engine: String,
}

/// Render error.
#[derive(Debug, Error)]
pub enum RenderError {
    #[error("engine error: {0}")]
    Engine(String),
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("core error: {0}")]
    Core(#[from] somadoc_core::CoreError),
}

/// `RenderResult` alias.
pub type RenderResult = Result<RenderOutput, RenderError>;

/// Rendering engine trait — implemented by all concrete engines.
#[async_trait]
pub trait RenderEngine: Send + Sync {
    /// The engine's unique id (e.g. "typst").
    fn id(&self) -> &'static str;

    /// Supported input formats (e.g. "markdown").
    fn supported_inputs(&self) -> &[&str];

    /// Supported output formats (e.g. "pdf").
    fn supported_outputs(&self) -> &[&str];

    /// Typeset (markdown / IR) into PDF bytes.
    async fn typeset(&self, req: &RenderRequest) -> RenderResult;

    /// Optional: typeset markdown into per-page SVG bytes (live preview).
    /// Unsupported by default.
    async fn typeset_svg(&self, _req: &RenderRequest) -> RenderResult {
        Err(RenderError::Engine(format!(
            "engine {} does not support live SVG preview",
            self.id()
        )))
    }
}
