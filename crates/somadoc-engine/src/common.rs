//! Common helpers for the Typst rendering path: slug validation, template
//! resolution, and Markdown frontmatter merging.

use std::collections::HashMap;

use crate::types::{RenderError, RenderRequest};

/// Validate a template slug: `[A-Za-z0-9_-]` only, to prevent path traversal.
pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && !slug.starts_with('-')
        && !slug.ends_with('-')
}

/// Merge `variables` into the Markdown YAML frontmatter.
///
/// If the content already has a frontmatter block (starting with `---`), the
/// variables are injected inside it; otherwise a new frontmatter block wraps the
/// content. Injected keys come first, so any key already written in the content
/// takes precedence (later wins).
fn merge_variables(content: &str, variables: &HashMap<String, String>) -> String {
    let fm_lines: Vec<String> = variables
        .iter()
        .map(|(k, v)| format!("{}: {:?}", k, v))
        .collect();
    if fm_lines.is_empty() {
        return content.to_string();
    }
    let fm_block = fm_lines.join("\n");

    let lines: Vec<&str> = content.lines().collect();
    let first_dash = lines.iter().position(|l| l.trim() == "---");
    match first_dash {
        Some(0) if lines.len() > 1 => format!("---\n{fm_block}\n{}", lines[1..].join("\n")),
        _ => format!("---\n{fm_block}\n---\n\n{content}"),
    }
}

/// Build the final Markdown from a render request (frontmatter merged).
pub fn build_markdown(req: &RenderRequest) -> Result<String, RenderError> {
    let merged = if let Some(c) = &req.content {
        if req.variables.is_empty() {
            c.clone()
        } else {
            merge_variables(c, &req.variables)
        }
    } else if let Some(value) = &req.ir {
        let ir: somadoc_core::ir::BlockNode = serde_json::from_value(value.clone())
            .map_err(|e| RenderError::Engine(format!("failed to parse IR: {e}")))?;
        let md_body = somadoc_core::to_markdown::ir_to_markdown(&ir);
        let mut fm: Vec<String> = Vec::new();
        for (k, v) in &req.variables {
            fm.push(format!("{}: {:?}", k, v));
        }
        if let Some(toc) = &req.toc {
            fm.push(format!("toc: {toc}"));
        }
        if fm.is_empty() {
            md_body
        } else {
            format!("---\n{}\n---\n\n{}", fm.join("\n"), md_body)
        }
    } else {
        return Err(RenderError::Engine(
            "request missing `content` or `ir`".into(),
        ));
    };
    Ok(sanitize_line_breaks(&merged))
}

/// Strip an explicit trailing `\\` from line-block (`|`-prefixed) lines.
fn sanitize_line_breaks(md: &str) -> String {
    let had_trailing_nl = md.ends_with('\n');
    let mut out = md
        .lines()
        .map(|l| {
            if l.trim_start().starts_with('|') {
                let t = l.trim_end();
                t.strip_suffix("\\\\").unwrap_or(t).trim_end().to_string()
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if had_trailing_nl {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("mainfont".to_string(), "Source Serif 4".to_string());
        m
    }

    #[test]
    fn merge_into_existing_frontmatter() {
        let c = "---\ntitle: Example\n---\n\nBody.\n";
        let out = merge_variables(c, &vars());
        assert!(
            out.contains("mainfont: \"Source Serif 4\""),
            "should inject mainfont: {out}"
        );
        assert!(out.contains("title: Example"));
        assert!(out.trim_start().starts_with("---"));
    }

    #[test]
    fn merge_generates_frontmatter_when_none() {
        let c = "# Title\n\nBody.\n";
        let out = merge_variables(c, &vars());
        assert!(
            out.trim_start().starts_with("---"),
            "should generate frontmatter: {out}"
        );
        assert!(out.contains("mainfont: \"Source Serif 4\""));
    }

    #[test]
    fn empty_variables_unchanged() {
        let c = "# Title\n";
        assert_eq!(merge_variables(c, &HashMap::new()), c);
    }

    #[test]
    fn sanitize_strips_latex_line_break_from_line_block() {
        let c = "| one\\\\\n| two\n| three\\\\   \n";
        let out = sanitize_line_breaks(c);
        assert_eq!(out, "| one\n| two\n| three\n");
    }
}
