//! Render Markdown to a PDF using the bundled reference template.
//!
//! Run from anywhere in the repository:
//!
//! ```text
//! cargo run -p somadoc-engine --example render_pdf
//! ```

use std::path::Path;

use somadoc_engine::types::{RenderEngine, RenderRequest};
use somadoc_engine::TypstEngine;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Resolve the template root against the crate directory so the example works
    // regardless of the current working directory.
    let templates_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    let engine = TypstEngine::new(templates_dir.to_string_lossy().into_owned());

    let markdown = "# Hello, somadoc\n\nThis is a paragraph with **bold** and *italic* text, a [link](https://example.com), and some inline math $E = mc^2$.\n\n- item one\n- item two\n";

    let req = RenderRequest {
        template_id: Some("soma".into()),
        content: Some(markdown.into()),
        ..Default::default()
    };

    let output = engine.typeset(&req).await?;
    std::fs::write("output.pdf", &output.bytes)?;
    println!(
        "Wrote output.pdf ({} bytes in {} ms)",
        output.bytes.len(),
        output.duration_ms
    );
    Ok(())
}
