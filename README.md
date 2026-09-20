# somadoc-engine

**A pure-Rust engine that turns Markdown into publish-grade documents.**

`somadoc-engine` compiles Markdown (plus handwritten LaTeX) into [Typst](https://typst.app) and renders it to **PDF** or **per-page SVG** — entirely in-process. No `pandoc`, no LaTeX distribution, no external sandbox.

## Why

- **Markdown-first** — write the Markdown you already know; the engine translates it to Typst for you.
- **Publish-grade** — real typesetting: hyphenation, justification, booktabs tables, footnotes, CJK support.
- **Self-contained** — pure Rust with no external tools; safe by construction (Typst has no command execution or arbitrary file access).
- **Incremental preview** — per-page SVG with a block-level source map for live, two-way editor ↔ preview positioning.

## Crates

| Crate | Description |
|-------|-------------|
| [`somadoc-core`](crates/somadoc-core) | IR data model, IR → Markdown serialization, text extraction |
| [`somadoc-engine`](crates/somadoc-engine) | Markdown → Typst → PDF / SVG engine |

## Quick start

```toml
[dependencies]
somadoc-engine = "0.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

```rust
use somadoc_engine::{RenderEngine, RenderRequest, TypstEngine};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Templates are resolved as `<dir>/<template_id>/template.typ`.
    let engine = TypstEngine::new("templates");

    let req = RenderRequest {
        template_id: Some("soma".into()),
        content: Some("# Hello\n\nA paragraph with **bold** text.".into()),
        ..Default::default()
    };

    let output = engine.typeset(&req).await?;
    std::fs::write("output.pdf", &output.bytes)?;
    Ok(())
}
```

`template_id` is optional: leave it out and the engine renders with its built-in default style. To use a custom layout, point `TypstEngine::new` at a directory containing `<template_id>/template.typ`. A complete reference template ships in this repository at [`crates/somadoc-engine/templates/soma`](crates/somadoc-engine/templates/soma) — copy it into your project, or pass that crate directory directly as shown in [`examples/render_pdf.rs`](crates/somadoc-engine/examples/render_pdf.rs).

### Live preview

`typeset_svg` returns a JSON envelope — `{"pages": ["<svg…>", …], "anchors": [{line, page, x, y, w, h}, …]}` — with one SVG per page plus a block-level source map for two-way editor ↔ preview positioning. Supplying a `session_id` reuses a persistent compilation context across requests so unchanged parts of the document are not re-laid-out.

## Features

- **Markdown → Typst** translation: headings, lists, tables, code blocks, blockquotes, inline math.
- **Handwritten LaTeX → Typst**: `\underline{...}`, `\mathcal{...}`, accents and more are translated.
- **Booktabs** three-line tables, footnotes, centered thematic breaks.
- **Per-page SVG** with block-level anchors (source map) for incremental preview and editor ↔ preview positioning.
- **Custom Typst templates** (`template.typ`) with YAML variable passthrough (`_somadoc_vars`).

## Templates

A template is a `template.typ` file that defines the entry point:

```typst
#let article(title: none, authors: (), lang: "en", toc: false, body) = {
  set page(paper: "a4", margin: 1in)
  body
}
```

The engine composes the translated Typst body with the template and injects YAML frontmatter variables as `_somadoc_vars`. Frontmatter keys `title`, `authors`, `lang` and `toc` are consumed by the engine; every other scalar key is forwarded to the template (a `-` in a key becomes `_`, e.g. `sender-name` → `sender_name`). See [`crates/somadoc-engine/templates/soma/template.typ`](crates/somadoc-engine/templates/soma/template.typ) for a complete example.

## Requirements

- Rust 1.85+ (stable, edition 2021).
- No system dependencies. Fonts are loaded from `typst-assets` and the OS font directories. Set `SOMADOC_FONTS_DIR` (colon-separated, like `PATH`) to restrict font scanning to specific directories instead of the whole system.

## Contributing

Contributions are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). For security reports, see [SECURITY.md](SECURITY.md).

## License

[Apache 2.0](LICENSE)
