//! The Typst rendering engine: a pure-Rust `markdown → typst → pdf` pipeline.
//!
//! Unlike pandoc + xelatex/tectonic, this engine **calls no external commands**:
//!   - parses Markdown into an AST with `comrak` (not pandoc)
//!   - converts the AST to Typst source with `mdxport` (including LaTeX math → Typst)
//!   - compiles **in-process** with the `typst` crate (`typst::compile` + `typst_pdf::pdf`), reusing the font World
//!
//! Benefits: stays in-process, no subprocess overhead, no pandoc/xelatex, natural incremental/font reuse.
//!
//! Template: when a request supplies a `template_id`, `<templates_dir>/<slug>/template.typ`
//! is used (a custom Typst template defining
//! `#let article(title: none, authors: (), lang: "en", toc: false, body)`); otherwise the
//! built-in default style is used. Fonts are set by the template via `set text(font: ...)`;
//! the in-process World scans the fonts bundled in `typst-assets` plus the OS font
//! directories (`SOMADOC_FONTS_DIR` restricts that scan to a whitelist of directories).

use std::path::Path;
use std::time::Instant;

use crate::types::{
    InlineImage, RenderEngine, RenderError, RenderOutput, RenderRequest, RenderResult,
};
use async_trait::async_trait;
use base64::Engine;
use typst::foundations::Bytes;

use crate::common::{build_markdown, valid_slug};
use crate::images::ImageBytes;
use crate::svg_out::AnchorSpec;

/// The Typst engine: Markdown → Typst → PDF/SVG (in-process `typst` compilation).
///
/// `templates_dir` is the root under which template packages are looked up: a
/// request with `template_id = Some("soma")` resolves to
/// `<templates_dir>/soma/template.typ`. An empty (or absent) `template_id`
/// renders with the built-in default style instead.
#[derive(Debug, Default)]
pub struct TypstEngine {
    templates_dir: String,
    timeout_ms: u64,
}

impl TypstEngine {
    /// Create an engine that resolves templates under `templates_dir`.
    pub fn new(templates_dir: impl Into<String>) -> Self {
        Self {
            templates_dir: templates_dir.into(),
            timeout_ms: 60_000,
        }
    }

    /// Override the per-render timeout (default: 60 s).
    pub fn with_timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    /// Validate the requested slug and confirm its `template.typ` exists.
    ///
    /// An empty slug is allowed and means "use the built-in default style".
    fn resolve_slug(&self, req: &RenderRequest) -> Result<String, RenderError> {
        let slug = req.template_id.clone().unwrap_or_default();
        if slug.is_empty() {
            return Ok(slug);
        }
        if !valid_slug(&slug) {
            return Err(RenderError::Engine(format!(
                "invalid template_id: {slug:?}"
            )));
        }
        let entry = format!("{}/{}", self.templates_dir, template_rel_typ(&slug));
        if !Path::new(&entry).exists() {
            return Err(RenderError::Engine(format!(
                "template `{slug}` not found (expected {entry})"
            )));
        }
        Ok(slug)
    }

    /// Read a template's `template.typ`. Returns `None` when no slug is given
    /// (callers then fall back to the built-in default style).
    fn read_custom_template(&self, slug: &str) -> Option<String> {
        if slug.is_empty() {
            return None;
        }
        let path = format!("{}/{}", self.templates_dir, template_rel_typ(slug));
        std::fs::read_to_string(path).ok()
    }
}

/// `.typ` template relative path (slug validated by `valid_slug`, no path traversal).
fn template_rel_typ(slug: &str) -> String {
    format!("{slug}/template.typ")
}

#[async_trait]
impl RenderEngine for TypstEngine {
    fn id(&self) -> &'static str {
        "typst"
    }

    fn supported_inputs(&self) -> &[&str] {
        &["markdown"]
    }

    fn supported_outputs(&self) -> &[&str] {
        &["pdf"]
    }

    async fn typeset(&self, req: &RenderRequest) -> RenderResult {
        let t0 = Instant::now();
        let slug = self.resolve_slug(req)?;

        // build markdown (merge variables into frontmatter; toc from req.toc)
        let markdown = build_markdown(req)?;
        let images = req.images.clone();

        // timeout guard + blocking render (mdxport/typst are synchronous CPU-bound; use spawn_blocking to avoid blocking the async executor)
        let templates_dir = self.templates_dir.clone();
        let timeout_ms = self.timeout_ms;
        let slug_for_task = slug.clone();
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            tokio::task::spawn_blocking(move || {
                let eng = TypstEngine {
                    templates_dir,
                    timeout_ms,
                };
                eng.render_blocking(&slug_for_task, &markdown, &images)
            }),
        )
        .await;

        let bytes = match result {
            Ok(Ok(Ok(bytes))) => bytes,
            Ok(Ok(Err(e))) => return Err(e),
            Ok(Err(join_err)) => {
                return Err(RenderError::Engine(format!(
                    "typesetting task failed: {join_err}"
                )))
            }
            Err(_) => {
                return Err(RenderError::Timeout(format!(
                    "{slug} typst typesetting timed out"
                )))
            }
        };

        let duration_ms = t0.elapsed().as_millis() as u64;
        Ok(RenderOutput {
            bytes,
            duration_ms,
            engine: self.id().to_string(),
        })
    }

    async fn typeset_svg(&self, req: &RenderRequest) -> RenderResult {
        let t0 = Instant::now();
        let slug = self.resolve_slug(req)?;
        let markdown = build_markdown(req)?;
        let session_id = req.session_id.clone().unwrap_or_default();
        let images = req.images.clone();

        let templates_dir = self.templates_dir.clone();
        let timeout_ms = self.timeout_ms;
        let slug_for_task = slug.clone();
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            tokio::task::spawn_blocking(move || {
                let eng = TypstEngine {
                    templates_dir,
                    timeout_ms,
                };
                if session_id.is_empty() {
                    eng.render_svg(&slug_for_task, &markdown, &images)
                } else {
                    eng.render_svg_incremental(&slug_for_task, &markdown, &session_id, &images)
                }
            }),
        )
        .await;

        let bytes = match result {
            Ok(Ok(Ok(bytes))) => bytes,
            Ok(Ok(Err(e))) => return Err(e),
            Ok(Err(join_err)) => {
                return Err(RenderError::Engine(format!(
                    "svg typesetting task failed: {join_err}"
                )))
            }
            Err(_) => {
                return Err(RenderError::Timeout(format!(
                    "{slug} typst svg typesetting timed out"
                )))
            }
        };
        Ok(RenderOutput {
            bytes,
            duration_ms: t0.elapsed().as_millis() as u64,
            engine: "typst-svg".to_string(),
        })
    }
}

impl TypstEngine {
    /// Render PDF bytes from markdown plus an optional custom `.typ` template.
    /// `custom_template: None` → mdxport's built-in default style.
    fn render_with_template_opt(
        markdown: &str,
        custom_template: Option<&str>,
        images: &[InlineImage],
    ) -> Result<Vec<u8>, RenderError> {
        match custom_template {
            Some(custom) => {
                let (src, logo, img_bytes, _anchors) =
                    Self::build_typst_source(markdown, custom, images)?;
                Self::compile_typst_pdf(&src, logo, &img_bytes)
            }
            // no custom template → reuse mdxport's high-level one-shot (built-in template + frontmatter variables)
            None => mdxport::markdown_to_pdf(markdown, &mdxport::Options::default())
                .map_err(|e| RenderError::Engine(format!("typst render failed: {e}"))),
        }
    }

    /// Render per-page SVG from markdown plus a custom `.typ` template (for live preview; requires a custom template.typ).
    fn render_svg_with_template_opt(
        markdown: &str,
        custom_template: Option<&str>,
        images: &[InlineImage],
    ) -> Result<Vec<u8>, RenderError> {
        let custom = custom_template.ok_or_else(|| {
            RenderError::Engine(
                "live preview only supports typst templates with template.typ".into(),
            )
        })?;
        let (src, logo, img_bytes, anchors) = Self::build_typst_source(markdown, custom, images)?;
        let (pages, source_map) =
            crate::svg_out::compile_typst_to_pages_with_anchors(&src, logo, &img_bytes, &anchors)
                .map_err(|e| RenderError::Engine(format!("typst svg compilation failed: {e}")))?;
        // Return JSON: {"pages":["<svg…>", …], "anchors":[{line,page,y}…]} so a preview client can render page by page and map positions two-way.
        let json = serde_json::json!({ "pages": pages, "anchors": source_map }).to_string();
        Ok(json.into_bytes())
    }

    /// Compile typst source → PDF bytes.
    /// Uses our own `svg_out::compile_typst_to_pdf` (reuses PagedDocument + the whitelisted font World),
    /// instead of mdxport's one-shot PDF (which hardcodes a full system-font scan and ignores SOMADOC_FONTS_DIR).
    fn compile_typst_pdf(
        typst_source: &str,
        logo: Option<Bytes>,
        images: &ImageBytes,
    ) -> Result<Vec<u8>, RenderError> {
        if std::env::var("SOMADOC_TYPST_DUMP").as_deref() == Ok("1") {
            let _ = std::fs::write(std::env::temp_dir().join("typst-last.typ"), typst_source);
        }
        crate::svg_out::compile_typst_to_pdf(typst_source, logo, images)
            .map_err(|e| RenderError::Engine(format!("typst compilation failed: {e}")))
    }

    /// Compose the final typst source from markdown (shared by the PDF and SVG paths; requires a custom template).
    /// Returns (source, optional logo bytes, inline image byte list).
    fn build_typst_source(
        markdown: &str,
        custom: &str,
        images: &[InlineImage],
    ) -> Result<(String, Option<Bytes>, ImageBytes, Vec<AnchorSpec>), RenderError> {
        use mdxport::{convert, frontmatter};

        // 1. split frontmatter (title/author/date/toc) + custom variables (sender-name etc.)
        let parsed = frontmatter::split_frontmatter(markdown)
            .map_err(|e| RenderError::Engine(format!("frontmatter parsing failed: {e}")))?;
        // 1. Split the body into blocks (on blank lines), recording each block's start line (body-relative, 1-based, matching the editor's line numbers).
        let blocks = split_into_blocks(parsed.body.trim_start_matches('\n'));
        let (vars, logo) = extract_logo(raw_frontmatter_map(markdown));
        // 2. inject block markers (standalone paragraphs), replaced with invisible #rect after later transforms (normalize/protect/mdxport).
        let marked_markdown = inject_block_markers_markdown(&blocks);
        // Pandoc line blocks (poetry verse: lines starting with `| `) → typst hard breaks (the `|` prefix is stripped, stanza blank lines kept)
        let body = normalize_line_blocks(&marked_markdown);

        // inline images → placeholders (`![alt](/somadoc-img/<id>)` → placeholder), collecting bytes + typst snippets.
        let (body, image_inject) = crate::images::protect_images(&body, images);

        // handwritten LaTeX commands → Typst: protect as placeholders (survive comrak escaping), then restore after conversion.
        let (body, latex_replacements) = crate::latex_commands::protect_markdown(&body);

        // 2. md → typst body (comrak AST)
        let mut converted = convert::convert_markdown_to_typst(
            &body,
            &parsed.frontmatter,
            &convert::ConvertOptions::default(),
        )
        .map_err(|e| RenderError::Engine(format!("markdown → typst conversion failed: {e}")))?;

        // restore the Typst snippets for inline images and handwritten LaTeX commands.
        converted.body = crate::images::restore_images(&converted.body, &image_inject.replacements);
        converted.body =
            crate::latex_commands::restore_placeholders(&converted.body, &latex_replacements);

        // booktabs three-line table bottom rule: mdxport's #table(...) output lacks the bottom rule.
        // Because Typst 0.13.1's table.cell element selector is unavailable at the template layer (unknown variable: cell),
        // a show rule can't add a bottom line to the last row — so here we inject table.hline(stroke: heavy) at the end of each top-level #table(...).
        converted.body = inject_table_bottom_rules(&converted.body);
        // Thematic break (---): centered and half-width.
        // mdxport emits a full-width `#line(length: 100%)`; this narrows it to 50%
        // (centering is handled by the template's `show line` rule).
        converted.body = normalize_thematic_breaks(&converted.body);

        // 3. replace block-marker text in the typst body with invisible #rect markers; returns (marked body, [(line, index)]).
        let (marked_body, anchor_specs) = replace_block_markers_with_rect(&converted.body, &blocks);

        // 4. apply template: custom template.typ + inject _somadoc_vars (YAML custom variables)
        let final_source = compose_document_with_vars(
            custom,
            &vars,
            converted.title.as_deref(),
            &converted.authors,
            &converted.lang,
            converted.toc,
            &marked_body,
        );

        // 5. locate each marker's byte range in the final source (for source-map Span resolution).
        let anchor_specs = locate_anchor_ranges(&final_source, anchor_specs);

        Ok((final_source, logo, image_inject.bytes, anchor_specs))
    }

    fn render_blocking(
        &self,
        slug: &str,
        markdown: &str,
        images: &[InlineImage],
    ) -> Result<Vec<u8>, RenderError> {
        let custom = self.read_custom_template(slug);
        Self::render_with_template_opt(markdown, custom.as_deref(), images)
    }

    /// For live preview: render markdown into per-page SVG bytes (only for templates with template.typ).
    pub fn render_svg(
        &self,
        slug: &str,
        markdown: &str,
        images: &[InlineImage],
    ) -> Result<Vec<u8>, RenderError> {
        let custom = self.read_custom_template(slug);
        Self::render_svg_with_template_opt(markdown, custom.as_deref(), images)
    }

    /// For incremental live preview: reuse the session's persistent World for incremental compilation (only for templates with template.typ).
    pub fn render_svg_incremental(
        &self,
        slug: &str,
        markdown: &str,
        session_id: &str,
        images: &[InlineImage],
    ) -> Result<Vec<u8>, RenderError> {
        let custom = self.read_custom_template(slug).ok_or_else(|| {
            RenderError::Engine(
                "live preview only supports typst templates with template.typ".into(),
            )
        })?;
        let (src, logo, img_bytes, anchors) = Self::build_typst_source(markdown, &custom, images)?;
        let (pages, source_map) =
            crate::svg_out::render_pages_incremental(session_id, &src, logo, &img_bytes, &anchors)
                .map_err(|e| {
                    RenderError::Engine(format!("typst svg incremental compilation failed: {e}"))
                })?;
        let json = serde_json::json!({ "pages": pages, "anchors": source_map }).to_string();
        Ok(json.into_bytes())
    }
}

/// Anchor spec: markdown line number (1-based) + the marker's byte range in the final typst source.
/// (defined in svg_out for source-map Span resolution; here it is only constructed internally)
/// A marker injected before a heading (a 1pt white-dot rect, nearly invisible on the page; its `#rect(...)` syntax Span is used for source-map resolution).
const ANCHOR_MARKER: &str = "#rect(width: 1pt, height: 1pt, fill: luma(100%))";

/// Block-marker prefix (injected into the markdown body; replaced with an invisible #rect after passing through mdxport).
/// Uppercase letters + digits only, to avoid mdxport's escape_text escaping special characters like `_`.
const BLOCK_MARKER_PREFIX: &str = "SOMADOCBLOCKANCHOR";

/// Split the body into blocks on blank lines, returning each block's (1-based start line, block text). Empty blocks are skipped.
fn split_into_blocks(body: &str) -> Vec<(usize, String)> {
    let mut blocks = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut cur_start = 1usize;
    for (line, raw) in (1usize..).zip(body.split('\n')) {
        let l = raw.trim_end_matches('\r');
        if l.trim().is_empty() {
            if !cur.is_empty() {
                blocks.push((cur_start, cur.join("\n")));
                cur.clear();
            }
        } else {
            if cur.is_empty() {
                cur_start = line;
            }
            cur.push(l);
        }
    }
    if !cur.is_empty() {
        blocks.push((cur_start, cur.join("\n")));
    }
    blocks
}

/// Inject a `SOMADOC_BLOCK_ANCHOR_<i>` standalone-paragraph marker before each block (alphanumeric, survives all transforms and mdxport).
fn inject_block_markers_markdown(blocks: &[(usize, String)]) -> String {
    let mut out = String::new();
    for (i, (_, block)) in blocks.iter().enumerate() {
        out.push_str(BLOCK_MARKER_PREFIX);
        out.push_str(&i.to_string());
        out.push_str("\n\n");
        out.push_str(block.trim_end_matches('\n'));
        out.push_str("\n\n");
    }
    out
}

/// Replace block-marker text in the typst body with invisible #rect markers; returns (marked body, [(block start line, index)]).
fn replace_block_markers_with_rect(
    body: &str,
    blocks: &[(usize, String)],
) -> (String, Vec<(usize, usize)>) {
    let mut out = String::with_capacity(body.len() + blocks.len() * 64);
    let mut specs = Vec::with_capacity(blocks.len());
    let mut remaining = body;
    for (i, (start_line, _)) in blocks.iter().enumerate() {
        let marker = format!("{BLOCK_MARKER_PREFIX}{i}");
        let Some(pos) = remaining.find(&marker) else {
            break; // marker lost (extremely rare) → stop; subsequent alignment is unreliable
        };
        out.push_str(&remaining[..pos]);
        out.push_str(ANCHOR_MARKER);
        out.push_str(" <somadoc-anchor-");
        out.push_str(&i.to_string());
        out.push('>');
        specs.push((*start_line, i));
        remaining = &remaining[pos + marker.len()..];
    }
    out.push_str(remaining);
    (out, specs)
}

/// Locate each anchor marker's `#rect(...)` byte range in the final source (for `Source::range` Span matching).
fn locate_anchor_ranges(final_source: &str, specs: Vec<(usize, usize)>) -> Vec<AnchorSpec> {
    specs
        .into_iter()
        .filter_map(|(line, idx)| {
            let label = format!("<somadoc-anchor-{idx}>");
            let label_pos = final_source.find(&label)?;
            let line_start = final_source[..label_pos]
                .rfind('\n')
                .map(|p| p + 1)
                .unwrap_or(0);
            let line_str = &final_source[line_start..label_pos];
            let rect_rel = line_str.find("#rect(")?;
            let rect_start = line_start + rect_rel;
            // the marker byte range spans from `#rect(` to the label start — covering the `#rect(...)` syntax (its Span falls inside).
            Some(AnchorSpec {
                line,
                byte_range: rect_start..label_pos,
            })
        })
        .collect()
}

/// Inject a `table.hline(...)` line at the end of each top-level `#table(` block in the body as the booktabs bottom rule.
/// Handle bracket nesting: find the `)` matching `#table(` and insert the hline before it.
fn inject_table_bottom_rules(body: &str) -> String {
    let mut out = String::with_capacity(body.len() + 64);
    let bytes: Vec<char> = body.chars().collect();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        // identify a top-level `#table(`
        if bytes[i..].starts_with(&['#', 't', 'a', 'b', 'l', 'e', '('][..])
            && (i == 0 || bytes[i - 1] != '\\')
        {
            // find the matching `)` (respecting paren balance)
            let mut depth = 0usize;
            let mut j = i + 7;
            let mut found: Option<usize> = None;
            while j < n {
                match bytes[j] {
                    '(' => depth += 1,
                    ')' => {
                        if depth == 0 {
                            found = Some(j);
                            break;
                        }
                        depth -= 1;
                    }
                    _ => {}
                }
                j += 1;
            }
            if let Some(end) = found {
                out.extend(&bytes[i..end]);
                // insert the hline line before the matching `)` (booktabs bottom rule as the table's last argument; separated by a newline, no leading comma)
                out.push_str("\n  table.hline(stroke: 1pt + luma(60)),");
                out.push(')');
                i = end + 1;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// Thematic break (---): turn mdxport's full-width `#line(length: 100%, stroke: 0.5pt)` into a
/// half-width `#line(length: 50%, stroke: 0.5pt)`.
fn normalize_thematic_breaks(body: &str) -> String {
    body.replace(
        "#line(length: 100%, stroke: 0.5pt)",
        "#line(length: 50%, stroke: 0.5pt)",
    )
}

/// Read the Markdown YAML frontmatter (the raw `---` … `---` block) into a string map.
/// Keep only **scalar string/number/bool** values (lists/nested objects are ignored); parsing failures return empty (without blocking rendering).
/// Used to pass template custom variables like `sender-name` to `template.typ` (via `_somadoc_vars`).
fn raw_frontmatter_map(markdown: &str) -> Vec<(String, String)> {
    let normalized = markdown.trim_start_matches('\u{feff}');
    let mut lines = normalized.lines();
    if lines.next() != Some("---") {
        return Vec::new();
    }
    let mut block = String::new();
    let mut found_end = false;
    for line in lines {
        if line == "---" {
            found_end = true;
            break;
        }
        block.push_str(line);
        block.push('\n');
    }
    if !found_end {
        return Vec::new();
    }
    let Ok(map) = serde_yaml::from_str::<serde_yaml::Mapping>(&block) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (k, v) in map {
        let key = match k.as_str() {
            Some(s) => s,
            None => continue,
        };
        // skip the standard fields already consumed by mdxport; pass through only custom variables.
        // Note: keep `date` and `author` (templates may use author/date for signatures).
        match key {
            "title" | "authors" | "lang" | "toc" => continue,
            _ => {}
        }
        let value = match &v {
            serde_yaml::Value::String(s) => s.clone(),
            serde_yaml::Value::Number(n) => n.to_string(),
            serde_yaml::Value::Bool(b) => b.to_string(),
            _ => continue,
        };
        out.push((key.to_string(), value));
    }
    out
}

/// Extract `logo` (base64) from frontmatter variables and decode it into bytes; returns (remaining variables, optional logo bytes).
/// When a logo is present, additionally inject a `logo: "1"` flag so templates can check `_v("logo")` to decide whether to render the image.
fn extract_logo(vars: Vec<(String, String)>) -> (Vec<(String, String)>, Option<Bytes>) {
    let mut logo_b64: Option<String> = None;
    let mut out = Vec::new();
    for (k, v) in vars {
        if k == "logo" {
            logo_b64 = Some(v);
        } else if k == "logo_format" {
            // ignore: typst image() auto-detects the image format
        } else {
            out.push((k, v));
        }
    }
    let logo = logo_b64
        .and_then(|b64| {
            base64::engine::general_purpose::STANDARD
                .decode(b64.as_bytes())
                .ok()
        })
        .map(Bytes::new);
    if logo.is_some() {
        out.push(("logo".to_string(), "1".to_string()));
    }
    (out, logo)
}

/// Escape a string into a Typst double-quoted string literal (quotes/backslashes within the value).
fn escape_vars_string(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Turn a frontmatter variable name into a Typst dictionary key (`-` becomes `_`).
///
/// Returns `None` when the name cannot be represented as a Typst identifier.
/// Keys come from user-authored frontmatter, so rejecting anything outside
/// `[A-Za-z0-9_]` keeps them from injecting arbitrary Typst code into the
/// generated document.
fn typst_dict_key(name: &str) -> Option<String> {
    let key = name.replace('-', "_");
    let valid = !key.is_empty()
        && !key.starts_with(|c: char| c.is_ascii_digit())
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    valid.then_some(key)
}

/// Compose a Typst document from a custom `template.typ`: inject a
/// `#let _somadoc_vars = (...)` dictionary before the template source (YAML
/// custom variables, with `-` in keys converted to `_`), then append the
/// `#article(...)[body]` call.
fn compose_document_with_vars(
    template: &str,
    vars: &[(String, String)],
    title: Option<&str>,
    authors: &[String],
    lang: &str,
    toc: bool,
    body: &str,
) -> String {
    let title_value = title.filter(|v| !v.trim().is_empty()).map_or_else(
        || "none".to_string(),
        |v| format!("\"{}\"", escape_vars_string(v)),
    );

    let authors_value = if authors.is_empty() {
        "()".to_string()
    } else {
        let formatted = authors
            .iter()
            .map(|a| format!("\"{}\"", escape_vars_string(a)))
            .collect::<Vec<_>>()
            .join(", ");
        if authors.len() == 1 {
            format!("({formatted},)")
        } else {
            format!("({formatted})")
        }
    };

    // Inject the vars dictionary (even when empty, so templates can always
    // reference `_somadoc_vars` without error). Keys that cannot be represented
    // as Typst identifiers are dropped.
    // Note: an empty dict must be written as `(:)` — an empty tuple `()` is an
    // array, and `.at("key")` on it would fail with "expected integer".
    let entries: Vec<(String, &str)> = vars
        .iter()
        .filter_map(|(k, v)| typst_dict_key(k).map(|key| (key, v.as_str())))
        .collect();
    let mut source = String::new();
    if entries.is_empty() {
        source.push_str("#let _somadoc_vars = (:)\n\n");
    } else {
        source.push_str("#let _somadoc_vars = (");
        for (i, (key, value)) in entries.iter().enumerate() {
            if i > 0 {
                source.push_str(", ");
            }
            source.push_str(key);
            source.push_str(": \"");
            source.push_str(&escape_vars_string(value));
            source.push('"');
        }
        source.push_str(")\n\n");
    }
    source.push_str(template);
    source.push_str("\n\n");
    source.push_str(&format!(
        "#article(title: {title_value}, authors: {authors_value}, lang: \"{}\", toc: {toc})[",
        escape_string(lang),
    ));
    source.push('\n');
    source.push_str(body);
    source.push('\n');
    source.push_str("]\n");
    source
}

/// Escape lang etc. (mirrors mdxport) — only newlines and quotes/backslashes.
fn escape_string(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
}

/// Decide whether a string (after stripping the `|` prefix) is a pipe-table separator row (e.g. ` --- | --- |`):
/// contains only `| - : whitespace` and is non-empty → it is a header separator row.
fn rest_is_table_sep(rest: &str) -> bool {
    if rest.trim().is_empty() {
        return false;
    }
    !rest
        .chars()
        .any(|c| !matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

/// Pandoc line blocks → markdown that comrak/mdxport can render as typst hard breaks.
/// Line block: consecutive lines starting with `| ` (one per line); a lone `|` line marks an empty line within the block (a stanza break).
/// The poetry template uses it to drive LaTeX `verse` layout; mdxport/comrak don't recognize Pandoc line blocks,
/// and would concatenate `| ` as literals — so here we strip the `| ` prefix and add a hard break `\\` at the end of each line,
/// keeping empty lines as stanza breaks. Detection: consecutive `|`-prefixed lines with no inline `|` separator (to avoid touching pipe tables).
fn normalize_line_blocks(markdown: &str) -> String {
    let lines: Vec<&str> = markdown.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    let n = lines.len();
    while i < n {
        let line = lines[i];
        // is this a line-block line: starts with `|` or `| ` (leading whitespace allowed)
        let trimmed = line.trim_start();
        if trimmed == "|" || trimmed.starts_with("| ") || trimmed.starts_with("|\t") {
            // collect the maximal run of consecutive lines for this line block
            let start = i;
            let mut j = i;
            let mut has_sep = false;
            while j < n {
                let t = lines[j].trim_start();
                if t == "|" || t.starts_with("| ") || t.starts_with("|\t") {
                    // pipe-table separator row (e.g. |---| or |:--:|) → this group is a table, keep as-is
                    if rest_is_table_sep(&t[1..]) {
                        has_sep = true;
                    }
                    j += 1;
                } else {
                    break;
                }
            }
            if has_sep {
                // it is a table → keep as-is, continue
                for ln in &lines[start..j] {
                    out.push((*ln).to_string());
                }
                i = j;
                continue;
            }
            // line block: strip `|`, add a hard break `\\` at the end of each line (only when another verse line follows in the same stanza; not at line/stanza ends, to avoid a literal backslash)
            for (idx, ln) in lines[start..j].iter().enumerate() {
                let t = ln.trim_start();
                if t == "|" {
                    out.push(String::new()); // empty line within the block → stanza break
                    continue;
                }
                let content = t[1..].trim_start_matches([' ', '\t']);
                // is the next line a verse line in the same stanza (a non-empty block line) — add the hard break only at line ends, not stanza ends
                let has_next_verse = (start + idx + 1 < j) && {
                    let nt = lines[start + idx + 1].trim_start();
                    nt != "|" && (nt.starts_with("| ") || nt.starts_with("|\t"))
                };
                if has_next_verse {
                    out.push(format!("{content}\\"));
                } else {
                    out.push(content.to_string());
                }
            }
            i = j;
        } else {
            out.push(line.to_string());
            i += 1;
        }
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal CJK-printable Typst template (defines the article contract; IBM Plex Sans SC is in the system fonts).
    fn zh_template() -> String {
        r#"#let article(title: none, authors: (), lang: "zh", toc: false, body) = {
  set text(font: ("IBM Plex Sans SC", "Noto Sans CJK SC"), size: 12pt, lang: lang)
  set page(paper: "a4", margin: 24mm)
  set par(justify: true)
  if title != none {
    text(size: 18pt, weight: "bold", title)
  }
  body
}
"#
        .to_string()
    }

    #[test]
    fn compiles_markdown_to_pdf_default_template() {
        let markdown = "# 标题\n\n正文，带 **加粗** 与 $E = mc^2$。\n\n- 项一\n- 项二\n";
        let bytes = TypstEngine::render_with_template_opt(markdown, None, &[])
            .expect("default template should render");
        assert!(bytes.starts_with(b"%PDF"), "should produce a %PDF header");
        assert!(bytes.len() > 500, "PDF should be non-empty");
    }

    #[test]
    fn compiles_markdown_with_custom_template_and_cjk() {
        let markdown = "# 中文标题\n\n这是一段中文，含**加粗**与列表。\n\n- 甲\n- 乙\n";
        let bytes = TypstEngine::render_with_template_opt(markdown, Some(&zh_template()), &[])
            .expect("custom template should render");
        assert!(bytes.starts_with(b"%PDF"));
    }

    #[test]
    fn split_into_blocks_records_start_lines() {
        let body = "# 甲\n\n正文。\n\n- 列表\n- 项\n";
        let blocks = split_into_blocks(body);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].0, 1); // the heading block
        assert_eq!(blocks[1].0, 3); // the paragraph block
        assert_eq!(blocks[2].0, 5); // the list block
        assert_eq!(blocks[2].1, "- 列表\n- 项");
    }

    #[test]
    fn render_svg_produces_block_anchors() {
        let markdown =
            "# 第一节\n\n第一节正文。\n\n## 第二节\n\n第二节正文。\n\n### 第三节\n\n第三节正文。\n";
        let bytes = TypstEngine::render_svg_with_template_opt(markdown, Some(&zh_template()), &[])
            .expect("svg render should succeed");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("should be JSON");
        let anchors = json["anchors"].as_array().expect("should have anchors");
        assert_eq!(
            anchors.len(),
            6,
            "should extract 6 block anchors (3 headings + 3 paragraphs)"
        );
        for a in anchors {
            assert!(a["line"].is_u64(), "anchor should have line");
            assert!(a["page"].is_u64(), "anchor should have page");
            for k in ["y", "x", "w", "h"] {
                let v = a[k]
                    .as_f64()
                    .expect("anchor rectangle ratio should be a number");
                assert!((0.0..=1.0).contains(&v), "{k} should be within 0..1");
            }
        }
        // anchors are ordered by increasing line number
        let lines: Vec<u64> = anchors
            .iter()
            .map(|a| a["line"].as_u64().unwrap())
            .collect();
        assert!(
            lines.windows(2).all(|w| w[0] < w[1]),
            "anchor line numbers should increase"
        );
    }

    #[test]
    fn translates_raw_latex_commands_to_typst() {
        let markdown = "# 标题\n\n这段用 \\textbf{加粗}、\\color{#ff70b7}{粉色}、\\large{大字} 与 \\utilde{下划线} 测试。\n\n\\mathsf{\\color{#ff70b7}{\\normalsize  \\utilde{~~~@2025~~linnon~lab~~~}}}\n";
        let bytes = TypstEngine::render_with_template_opt(markdown, Some(&zh_template()), &[])
            .expect("document with handwritten LaTeX commands should render");
        assert!(bytes.starts_with(b"%PDF"), "should produce a %PDF header");
        assert!(bytes.len() > 500, "PDF should be non-empty");
    }

    #[test]
    fn nested_markdown_inside_latex_command_compiles() {
        let markdown = "# 标题\n\n\\underline{例如**强调一段同时含 *斜体* 与`代码`的句子**。}\n";
        let bytes = TypstEngine::render_with_template_opt(markdown, Some(&zh_template()), &[])
            .expect("handwritten LaTeX command with nested Markdown should render");
        assert!(bytes.starts_with(b"%PDF"), "should produce a %PDF header");
        assert!(bytes.len() > 500, "PDF should be non-empty");
    }

    #[test]
    fn renders_logo_from_base64() {
        // a 1x1 transparent PNG (base64), verifying the full chain: logo decoded from frontmatter → World registration → image() reference.
        let b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";
        let markdown = format!("---\nlogo: {b64}\n---\n\n# 标题\n\n正文。\n");
        let tpl = r#"#let article(title: none, authors: (), lang: "zh", toc: false, body) = {
  set page(paper: "a4", margin: 24mm)
  if _somadoc_vars.at("logo", default: "") != "" {
    image("/somadoc-logo", width: 4em)
  }
  body
}
"#;
        let bytes = TypstEngine::render_with_template_opt(&markdown, Some(tpl), &[])
            .expect("template with logo should render");
        assert!(bytes.starts_with(b"%PDF"), "should produce a %PDF header");
        assert!(bytes.len() > 500);
    }

    #[test]
    fn injects_table_bottom_rules_for_booktabs() {
        // inject a table.hline bottom rule into each top-level #table(...) (the 3rd line of a booktabs three-line table)
        let body = "前文。\n\n#table(\n  columns: 2,\n  table.header[列一][列二],\n  [甲], [乙],\n)\n\n后文。\n";
        let out = inject_table_bottom_rules(body);
        assert!(
            out.contains("table.hline"),
            "should inject bottom rule: {out}"
        );
        assert!(out.contains("table.hline(stroke: 1pt + luma(60))"));
        // the bottom rule should be before the table's `)` (a comma follows hline, then the closing paren)
        let after_hline = out
            .split("table.hline(stroke: 1pt + luma(60))")
            .nth(1)
            .unwrap_or("");
        let after = after_hline.trim_start_matches(',').trim_start();
        assert!(
            after.starts_with(')'),
            "hline should be followed by the table's closing paren: {out}"
        );
    }

    #[test]
    fn normalize_line_blocks_converts_verse_to_hard_breaks() {
        let md = "| 灯影落进纸间的湖\n| 风替夜合上最后一页\n|\n| 新茶浮起半枚月光\n";
        let out = normalize_line_blocks(md);
        // strip `| `, add a hard break at the end of each line (only when another verse line follows in the same stanza); empty lines are kept as stanza breaks
        let expected = "灯影落进纸间的湖\\\n风替夜合上最后一页\n\n新茶浮起半枚月光\n";
        assert_eq!(
            out, expected,
            "line block conversion error, actual output: {out:?}"
        );
        // pipe tables are not touched (a whole group with a |-|- separator row is kept as-is)
        let table = "| 列一 | 列二 |\n| --- | --- |\n| 甲 | 乙 |\n";
        assert_eq!(normalize_line_blocks(table), table);
        // non-line-block text is kept as-is
        let normal = "# 标题\n\n正文段落。\n";
        assert_eq!(normalize_line_blocks(normal), normal);
    }

    #[test]
    fn raw_frontmatter_map_keeps_custom_vars() {
        let md = "---\nsender-name: 江疏桐\nsender-org: 吴越丝绸研究所\nrecipient-title: 运营总监\ntitle: 关于函\nauthor: 林清远\nlang: zh\ntoc: false\ndate: 2026年8月10日\n---\n\n正文\n";
        let map = raw_frontmatter_map(md);
        // custom variables are kept (hyphens preserved); only the engine's standard control fields title/authors/lang/toc are removed.
        // author/date are kept (some templates use author/date for signatures).
        assert_eq!(map.len(), 5, "should keep 5 variables: {map:?}");
        assert!(map.contains(&("sender-name".to_string(), "江疏桐".to_string())));
        assert!(map.contains(&("sender-org".to_string(), "吴越丝绸研究所".to_string())));
        assert!(map.contains(&("recipient-title".to_string(), "运营总监".to_string())));
        assert!(map.contains(&("author".to_string(), "林清远".to_string())));
        assert!(map.contains(&("date".to_string(), "2026年8月10日".to_string())));
        assert!(!map
            .iter()
            .any(|(k, _)| k == "title" || k == "lang" || k == "toc"));
        // no frontmatter → empty
        assert!(raw_frontmatter_map("# 没有 frontmatter\n").is_empty());
    }

    #[test]
    fn no_placeholder_leaks_into_the_composed_source() {
        // Regression: handwritten LaTeX inside a math span used to leak "SOMADOCLATEXCMD…"
        // into the generated Typst source, because mdxport's math converter separates
        // identifier characters with spaces and `restore_placeholders` could no longer match.
        let markdown = "行内 $e^{-x^2}\\,dx$ 与块级：\n\n$$\n\\int_0^\\infty e^{-x^2}\\,dx = \\frac{\\sqrt{\\pi}}{2}\n$$\n\n\\textcolor{#1E88E5}{蓝}。\n";
        let (source, _logo, _images, _anchors) =
            TypstEngine::build_typst_source(markdown, &zh_template(), &[]).expect("compose");
        assert!(
            !source.contains("SOMADOC"),
            "no placeholder may survive into the generated source:\n{source}"
        );
    }

    #[test]
    fn compose_document_with_vars_injects_dict_and_escapes() {
        let tpl =
            "#let article(title: none, authors: (), lang: \"zh\", toc: false, body) = { body }";
        let vars = vec![
            ("sender-name".to_string(), "江\"疏\"桐".to_string()),
            ("sender-org".to_string(), "研究所".to_string()),
        ];
        let src = compose_document_with_vars(tpl, &vars, None, &[], "zh", false, "正文内容");
        // hyphen → underscore
        assert!(
            src.contains(
                "_somadoc_vars = (sender_name: \"江\\\"疏\\\"桐\", sender_org: \"研究所\")"
            ),
            "vars dict error: {src}"
        );
        // template source + call
        assert!(src.contains(tpl));
        assert!(src.contains("#article(title: none, authors: (), lang: \"zh\", toc: false)["));
        assert!(src.contains("正文内容"));
        // no variables → still inject an empty dictionary (as `(:)`; an empty tuple `()` would be treated as an array, causing .at("key") to error)
        let empty = compose_document_with_vars(tpl, &[], None, &[], "zh", false, "b");
        assert!(
            empty.starts_with("#let _somadoc_vars = (:"),
            "should inject an empty dict vars: {empty}"
        );
    }
}
