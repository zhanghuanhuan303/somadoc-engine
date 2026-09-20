//! Plain-text extraction.
//! Collects the text and code values of every node in a document.

use crate::ir::{BlockNode, InlineNode};

pub fn extract_text(ir: &BlockNode) -> String {
    let mut out = String::new();
    match ir {
        BlockNode::Document { children } => {
            for c in children {
                collect_block(c, &mut out);
            }
        }
        other => collect_block(other, &mut out),
    }
    out
}

fn collect_block(node: &BlockNode, out: &mut String) {
    match node {
        BlockNode::Paragraph { children }
        | BlockNode::Heading { children, .. }
        | BlockNode::TableCell { children } => {
            for n in children {
                collect_inline(n, out);
            }
        }
        BlockNode::BlockQuote { children } => {
            for c in children {
                collect_block(c, out);
            }
        }
        BlockNode::CodeBlock { value, .. } => out.push_str(value),
        BlockNode::List { children, .. } => {
            for c in children {
                collect_block(c, out);
            }
        }
        BlockNode::ListItem { children } => {
            for c in children {
                collect_block(c, out);
            }
        }
        BlockNode::Table { children, .. } => {
            for c in children {
                collect_block(c, out);
            }
        }
        BlockNode::TableRow { children, .. } => {
            for c in children {
                collect_block(c, out);
            }
        }
        BlockNode::RawBlock { value, .. } => out.push_str(value),
        _ => {}
    }
}

fn collect_inline(node: &InlineNode, out: &mut String) {
    match node {
        InlineNode::Text { value } => out.push_str(value),
        InlineNode::Code { value } => out.push_str(value),
        InlineNode::Strong { children }
        | InlineNode::Emphasis { children }
        | InlineNode::Strikethrough { children } => {
            for n in children {
                collect_inline(n, out);
            }
        }
        InlineNode::Link { children, .. } => {
            for n in children {
                collect_inline(n, out);
            }
        }
        _ => {}
    }
}
