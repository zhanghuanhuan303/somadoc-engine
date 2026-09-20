//! IR → Markdown serialization.
//! Rebuilds Markdown from the IR, preallocating the per-block output buffers so
//! that large documents serialize without repeated reallocation.

use crate::ir::{BlockNode, InlineNode};

/// Render a whole document to Markdown.
pub fn ir_to_markdown(ir: &BlockNode) -> String {
    let children = match ir {
        BlockNode::Document { children } => children,
        other => std::slice::from_ref(other),
    };
    let mut out: Vec<String> = Vec::with_capacity(children.len());
    for block in children {
        out.push(render_block(block));
    }
    out.join("\n\n")
}

fn render_block(node: &BlockNode) -> String {
    match node {
        BlockNode::Heading { level, children } => {
            format!("{} {}", "#".repeat(*level as usize), inline(children))
        }
        BlockNode::Paragraph { children } => inline(children),
        BlockNode::BlockQuote { children } => children
            .iter()
            .map(|c| format!("> {}", render_block(c)))
            .collect::<Vec<_>>()
            .join("\n"),
        BlockNode::CodeBlock { lang, value } => {
            format!("```{}\n{}\n```", lang.clone().unwrap_or_default(), value)
        }
        BlockNode::List {
            ordered,
            start,
            children,
        } => {
            let mut items: Vec<String> = Vec::with_capacity(children.len());
            for (i, li) in children.iter().enumerate() {
                // Take content from the listItem's children
                let body = match li {
                    BlockNode::ListItem { children } => list_item_body(children),
                    other => render_block(other),
                };
                if *ordered {
                    let num = start.map(|s| s + i as u32).unwrap_or(i as u32 + 1);
                    items.push(format!("{}. {}", num, body));
                } else {
                    items.push(format!("- {}", body));
                }
            }
            items.join("\n")
        }
        BlockNode::ListItem { children } => format!("- {}", list_item_body(children)),
        BlockNode::Table {
            children, columns, ..
        } => render_table(children, columns),
        BlockNode::ThematicBreak => "---".to_string(),
        BlockNode::ImageBlock { src, alt, .. } => {
            format!("![{}]({})", alt.clone().unwrap_or_default(), src)
        }
        BlockNode::RawBlock { value, .. } => value.clone(),
        BlockNode::Document { .. } | BlockNode::TableRow { .. } | BlockNode::TableCell { .. } => {
            String::new()
        }
    }
}

/// Render a listItem's content (no duplicate prefix for the list marker).
fn list_item_body(nodes: &[BlockNode]) -> String {
    nodes.iter().map(render_block).collect::<Vec<_>>().join(" ")
}

fn render_table(rows: &[BlockNode], columns: &serde_json::Value) -> String {
    let mut md_rows: Vec<String> = Vec::with_capacity(rows.len());
    for row in rows {
        if let BlockNode::TableRow { children, .. } = row {
            let cells: Vec<String> = children
                .iter()
                .filter_map(|c| match c {
                    BlockNode::TableCell { children } => Some(inline(children)),
                    _ => None,
                })
                .collect();
            if !cells.is_empty() {
                md_rows.push(format!("| {} |", cells.join(" | ")));
            }
        }
    }
    if md_rows.is_empty() {
        return String::new();
    }
    // Header separator row
    let _ = columns; // Alignment info is not expanded for now; the separator row follows the first row's column count.
    let header_cells = md_rows[0]
        .split('|')
        .filter(|s| !s.trim().is_empty())
        .count();
    let divider = format!("| {} |", vec!["---"; header_cells].join(" | "));
    let mut out = vec![md_rows[0].clone(), divider];
    out.extend(md_rows.iter().skip(1).cloned());
    out.join("\n")
}

fn inline(nodes: &[InlineNode]) -> String {
    let mut out = String::new();
    for n in nodes {
        out.push_str(&render_inline(n));
    }
    out
}

fn render_inline(node: &InlineNode) -> String {
    match node {
        InlineNode::Text { value } => value.clone(),
        InlineNode::Strong { children } => format!("**{}**", inline(children)),
        InlineNode::Emphasis { children } => format!("*{}*", inline(children)),
        InlineNode::Code { value } => format!("`{}`", value),
        InlineNode::Link { href, children, .. } => {
            format!("[{}]({})", inline(children), href)
        }
        InlineNode::Image { src, alt, .. } => {
            format!("![{}]({})", alt.clone().unwrap_or_default(), src)
        }
        InlineNode::FootnoteRef { identifier } => format!("[^{}]", identifier),
        InlineNode::Strikethrough { children } => format!("~~{}~~", inline(children)),
    }
}
