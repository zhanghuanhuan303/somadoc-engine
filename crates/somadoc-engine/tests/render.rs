//! End-to-end tests against the bundled reference template.

use std::path::Path;

use somadoc_engine::types::{RenderEngine, RenderRequest};
use somadoc_engine::TypstEngine;

/// Engine pointed at the reference template shipped with this crate.
fn engine() -> TypstEngine {
    let templates_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    TypstEngine::new(templates_dir.to_string_lossy().into_owned())
}

fn request(markdown: &str) -> RenderRequest {
    RenderRequest {
        template_id: Some("soma".into()),
        content: Some(markdown.to_string()),
        ..Default::default()
    }
}

#[tokio::test]
async fn bundled_template_renders_pdf() {
    let markdown = "# 标题\n\n正文，带 **加粗**、*斜体* 与 $E = mc^2$。\n\n- 甲\n- 乙\n";
    let out = engine()
        .typeset(&request(markdown))
        .await
        .expect("render PDF");
    assert!(out.bytes.starts_with(b"%PDF"), "output should be a PDF");
    assert!(out.bytes.len() > 1000, "PDF should have content");
    assert_eq!(out.engine, "typst");
}

#[tokio::test]
async fn bundled_template_renders_svg_pages_with_anchors() {
    let markdown = "# 第一节\n\n第一节正文。\n\n## 第二节\n\n第二节正文。\n";
    let out = engine()
        .typeset_svg(&request(markdown))
        .await
        .expect("render SVG");

    let json: serde_json::Value = serde_json::from_slice(&out.bytes).expect("JSON envelope");
    let pages = json["pages"].as_array().expect("pages array");
    assert!(!pages.is_empty(), "should produce at least one page");
    assert!(
        pages[0]
            .as_str()
            .expect("page is a string")
            .contains("<svg"),
        "pages should hold SVG documents"
    );

    let anchors = json["anchors"].as_array().expect("anchors array");
    assert_eq!(anchors.len(), 4, "2 headings + 2 paragraphs");
    let lines: Vec<u64> = anchors
        .iter()
        .map(|a| a["line"].as_u64().expect("line number"))
        .collect();
    assert!(
        lines.windows(2).all(|w| w[0] < w[1]),
        "anchor line numbers should increase: {lines:?}"
    );
}

#[tokio::test]
async fn omitted_template_uses_default_style() {
    let req = RenderRequest {
        content: Some("# Heading\n\nBody.".into()),
        ..Default::default()
    };
    let out = engine().typeset(&req).await.expect("render default style");
    assert!(out.bytes.starts_with(b"%PDF"));
}

#[tokio::test]
async fn unknown_template_is_reported() {
    let req = RenderRequest {
        template_id: Some("no-such-template".into()),
        content: Some("# Heading".into()),
        ..Default::default()
    };
    let err = engine().typeset(&req).await.expect_err("should fail");
    assert!(err.to_string().contains("not found"), "unexpected: {err}");
}

#[tokio::test]
async fn path_traversal_slug_is_rejected() {
    let req = RenderRequest {
        template_id: Some("../soma".into()),
        content: Some("# Heading".into()),
        ..Default::default()
    };
    let err = engine().typeset(&req).await.expect_err("should fail");
    assert!(
        err.to_string().contains("invalid template_id"),
        "unexpected: {err}"
    );
}
