//! IR (unified intermediate representation) data model + serde deserialization.
//! Serializable to/from JSON for cross-process interop.

use serde::Deserialize;

/// Inline node.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InlineNode {
    #[serde(rename = "text")]
    Text { value: String },
    #[serde(rename = "strong")]
    Strong { children: Vec<InlineNode> },
    #[serde(rename = "emphasis")]
    Emphasis { children: Vec<InlineNode> },
    #[serde(rename = "code")]
    Code { value: String },
    #[serde(rename = "link")]
    Link {
        href: String,
        #[serde(default)]
        title: Option<String>,
        children: Vec<InlineNode>,
    },
    #[serde(rename = "image")]
    Image {
        src: String,
        #[serde(default)]
        alt: Option<String>,
        #[serde(default)]
        title: Option<String>,
    },
    #[serde(rename = "footnoteRef")]
    FootnoteRef { identifier: String },
    #[serde(rename = "strikethrough")]
    Strikethrough { children: Vec<InlineNode> },
}

/// Block node (including `rawBlock`).
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BlockNode {
    #[serde(rename = "document")]
    Document { children: Vec<BlockNode> },
    #[serde(rename = "paragraph")]
    Paragraph { children: Vec<InlineNode> },
    #[serde(rename = "heading")]
    Heading {
        level: u8,
        children: Vec<InlineNode>,
    },
    #[serde(rename = "blockquote")]
    BlockQuote { children: Vec<BlockNode> },
    #[serde(rename = "codeBlock")]
    CodeBlock {
        #[serde(default)]
        lang: Option<String>,
        value: String,
    },
    #[serde(rename = "list")]
    List {
        #[serde(default)]
        ordered: bool,
        #[serde(default)]
        start: Option<u32>,
        children: Vec<BlockNode>,
    },
    #[serde(rename = "listItem")]
    ListItem { children: Vec<BlockNode> },
    #[serde(rename = "table")]
    Table {
        #[serde(default = "default_columns")]
        columns: serde_json::Value,
        children: Vec<BlockNode>,
    },
    #[serde(rename = "tableRow")]
    TableRow {
        #[serde(default)]
        header: bool,
        children: Vec<BlockNode>,
    },
    #[serde(rename = "tableCell")]
    TableCell { children: Vec<InlineNode> },
    #[serde(rename = "thematicBreak")]
    ThematicBreak,
    #[serde(rename = "imageBlock")]
    ImageBlock {
        #[serde(default)]
        src: String,
        #[serde(default)]
        alt: Option<String>,
        #[serde(default)]
        caption: Option<String>,
    },
    #[serde(rename = "rawBlock")]
    RawBlock {
        #[serde(default)]
        format: String,
        value: String,
    },
}

fn default_columns() -> serde_json::Value {
    serde_json::json!([])
}
