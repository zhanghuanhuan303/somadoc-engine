//! Typst → SVG output (for live preview).
//!
//! mdxport only exposes `compile_typst_to_pdf` (which prints PDF bytes), not the laid-out
//! `PagedDocument`. Live preview needs SVG, so we build a lightweight `World` here (an in-process
//! global font cache, matching mdxport's font loading) for typst → `PagedDocument` → SVG.
//!
//! `typst_svg::svg_merged(document, padding)` merges the whole document into a **single SVG** (good for
//! live preview — one frame, no per-page assembly on the client).

use std::collections::HashMap;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime};
use typst::layout::{Abs, Frame, FrameItem, PagedDocument};
use typst::syntax::{FileId, Source, Span, VirtualPath};
use typst::text::{Font, FontBook, FontInfo};
use typst::utils::LazyHash;
use typst::{Library, World};

/// Compile typst source into a whole-document SVG (multiple pages merged into one frame, fixed vertical padding).
/// `logo`: optional injected image bytes (registered at virtual path `/somadoc-logo` for templates to reference via `image("/somadoc-logo")`).
pub fn compile_typst_to_svg(
    source: &str,
    logo: Option<Bytes>,
    images: &[(String, Bytes)],
) -> Result<String, TypstError> {
    let document = compile_document(source, logo, images)?;
    let padding = typst::layout::Abs::cm(0.5);
    let merged = typst_svg::svg_merged(&document, padding);
    drop(document);
    Ok(merged)
}

/// Compile typst source into **per-page** SVG (one independent SVG per page, A4 aspect).
/// Consumers typically lay the pages out vertically in a scrollable view (showing the first page by
/// default, with a gap between pages).
pub fn compile_typst_to_pages(
    source: &str,
    logo: Option<Bytes>,
    images: &[(String, Bytes)],
) -> Result<Vec<String>, TypstError> {
    let document = compile_document(source, logo, images)?;
    let pages: Vec<String> = document.pages.iter().map(typst_svg::svg).collect();
    drop(document);
    Ok(pages)
}

/// Anchor spec (source-map input): markdown line number (1-based) + the marker's byte range in the final source.
#[derive(Debug, Clone)]
pub struct AnchorSpec {
    pub line: usize,
    pub byte_range: Range<usize>,
}

/// Heading/block anchor (source map): an editor line → output page (0-based) + in-page rectangle ratio (x/y/w/h all 0..1).
#[derive(Debug, Clone, serde::Serialize)]
pub struct HeadingAnchor {
    /// Markdown line number (1-based, relative to the body).
    pub line: usize,
    /// Output page (0-based).
    pub page: usize,
    /// Block top (fraction of page height, 0..1).
    pub y: f64,
    /// Block left edge (fraction of page width, 0..1).
    pub x: f64,
    /// Block width (fraction of page width, 0..1).
    pub w: f64,
    /// Block height (fraction of page height, 0..1).
    pub h: f64,
}

/// Compile typst source → per-page SVG + heading anchors (source map).
/// `anchor_ranges`: `(markdown line number, marker byte range in the final source)`, provided by `typst` after injecting markers.
pub fn compile_typst_to_pages_with_anchors(
    source: &str,
    logo: Option<Bytes>,
    images: &[(String, Bytes)],
    anchor_specs: &[AnchorSpec],
) -> Result<(Vec<String>, Vec<HeadingAnchor>), TypstError> {
    let (document, src) = compile_document_with_source(source, logo, images)?;
    let anchors = extract_anchors(&document, &src, anchor_specs);
    let pages: Vec<String> = document.pages.iter().map(typst_svg::svg).collect();
    drop(document);
    Ok((pages, anchors))
}

/// Extract block anchors from a laid-out `PagedDocument`: walk each page's frame, collect geometry of all visual items (text/shape/image),
/// resolve each marker's (page, y) from its Span, then compute the bounding box of visual items between adjacent markers to get the block's inline rectangle (x,y,w,h).
fn extract_anchors(
    document: &PagedDocument,
    source: &Source,
    anchor_specs: &[AnchorSpec],
) -> Vec<HeadingAnchor> {
    if anchor_specs.is_empty() {
        return Vec::new();
    }
    // collect visual items per page (geometry + optional Span).
    let mut pages: Vec<Vec<VisualItem>> = Vec::with_capacity(document.pages.len());
    for page in &document.pages {
        let mut items = Vec::new();
        collect_visual_items(&page.frame, Abs::zero(), Abs::zero(), &mut items);
        pages.push(items);
    }

    // resolve marker → (line, page, y).
    let mut markers: Vec<(usize, usize, f64)> = Vec::new();
    for (page_idx, items) in pages.iter().enumerate() {
        let page_h = document.pages[page_idx].frame.size().y.to_pt();
        if page_h <= 0.0 {
            continue;
        }
        for it in items {
            let Some(span) = it.span else { continue };
            let Some(range) = source.range(span) else {
                continue;
            };
            for spec in anchor_specs {
                if range.start >= spec.byte_range.start && range.end <= spec.byte_range.end {
                    markers.push((spec.line, page_idx, (it.y.to_pt() / page_h).clamp(0.0, 1.0)));
                    break;
                }
            }
        }
    }
    markers.sort_by_key(|m| m.0);
    if markers.is_empty() {
        return Vec::new();
    }

    let is_marker_span = |span: Option<Span>| -> bool {
        let Some(span) = span else { return false };
        let Some(range) = source.range(span) else {
            return false;
        };
        anchor_specs
            .iter()
            .any(|spec| range.start >= spec.byte_range.start && range.end <= spec.byte_range.end)
    };

    // for each marker block, compute its content bounding box (visual items between adjacent markers, excluding the marker itself).
    let mut anchors = Vec::with_capacity(markers.len());
    for (idx, &(line, page_idx, y_ratio)) in markers.iter().enumerate() {
        let page = &document.pages[page_idx];
        let page_h = page.frame.size().y.to_pt();
        let page_w = page.frame.size().x.to_pt();
        if page_h <= 0.0 || page_w <= 0.0 {
            continue;
        }
        let top = y_ratio * page_h;
        let bottom = markers
            .get(idx + 1)
            .filter(|m| m.1 == page_idx)
            .map(|m| m.2 * page_h)
            .unwrap_or_else(|| detect_content_bottom(&pages[page_idx], top));
        let mut min_x = f64::MAX;
        let mut max_x = f64::MIN;
        let mut min_y = f64::MAX;
        let mut max_y = f64::MIN;
        for it in &pages[page_idx] {
            let iy = it.y.to_pt();
            if iy < top - 1.0 || iy > bottom {
                continue;
            }
            if is_marker_span(it.span) {
                continue;
            }
            let ix = it.x.to_pt();
            let iw = it.w.to_pt();
            let ih = it.h.to_pt();
            if iw <= 0.1 && ih <= 0.1 {
                continue;
            }
            min_x = min_x.min(ix);
            max_x = max_x.max(ix + iw);
            min_y = min_y.min(iy);
            max_y = max_y.max(iy + ih);
        }
        if min_x == f64::MAX {
            // fallback: no content items (empty block) → marker position + one line height
            min_x = 0.0;
            max_x = page_w;
            min_y = top;
            max_y = top + 12.0;
        }
        anchors.push(HeadingAnchor {
            line,
            page: page_idx,
            y: (min_y / page_h).clamp(0.0, 1.0),
            x: (min_x / page_w).clamp(0.0, 1.0),
            w: ((max_x - min_x) / page_w).clamp(0.0, 1.0),
            h: ((max_y - min_y) / page_h).clamp(0.0, 1.0),
        });
    }
    anchors
}

/// Geometry of a visual item (text/shape/image), used to compute a block's bounding box (inline rectangle).
struct VisualItem {
    span: Option<Span>,
    x: Abs,
    y: Abs,
    w: Abs,
    h: Abs,
}

/// Recursively walk a frame, collecting the geometry of all visual items (text/shape/image) with accumulated horizontal + vertical translation.
/// Page content (margins + flowing layout) is translation-only (no rotation/scaling), so we only accumulate tx/ty.
fn collect_visual_items(frame: &Frame, ox: Abs, oy: Abs, out: &mut Vec<VisualItem>) {
    for (pos, item) in frame.items() {
        let ix = ox + pos.x;
        let iy = oy + pos.y;
        match item {
            FrameItem::Shape(shape, span) => {
                let sz = shape.geometry.bbox_size();
                out.push(VisualItem {
                    span: Some(*span),
                    x: ix,
                    y: iy,
                    w: sz.x,
                    h: sz.y,
                });
            }
            FrameItem::Image(_, size, span) => {
                out.push(VisualItem {
                    span: Some(*span),
                    x: ix,
                    y: iy,
                    w: size.x,
                    h: size.y,
                });
            }
            FrameItem::Text(text) => {
                // a text run's y is the baseline, not the visual top; ascender ≈ 0.8em, line height ≈ 1.2em.
                let baseline = iy.to_pt();
                let size = text.size.to_pt();
                out.push(VisualItem {
                    span: None,
                    x: ix,
                    y: Abs::pt(baseline - size * 0.8),
                    w: text.width(),
                    h: Abs::pt(size * 1.2),
                });
            }
            FrameItem::Group(group) => {
                let gx = ix + group.transform.tx;
                let gy = iy + group.transform.ty;
                collect_visual_items(&group.frame, gx, gy, out);
            }
            FrameItem::Link(_, _) | FrameItem::Tag(_) => {}
        }
    }
}

/// Detect a block's content-end Y (the last block on a page): collect visual-item Ys in `[top, +∞)`, sort, then find the first large gap (content → footer/header),
/// and cut before the gap so the footer (page number and other template decorations) is excluded from the block bounding box. Returns a point (pt) coordinate.
fn detect_content_bottom(items: &[VisualItem], top_pt: f64) -> f64 {
    let mut ys: Vec<f64> = items
        .iter()
        .filter(|it| {
            let iy = it.y.to_pt();
            let iw = it.w.to_pt();
            let ih = it.h.to_pt();
            iy >= top_pt - 1.0 && (iw > 0.1 || ih > 0.1)
        })
        .map(|it| it.y.to_pt())
        .collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let Some(first) = ys.first().copied() else {
        return top_pt + 12.0;
    };
    // one line is ~14pt (12pt font); a gap > 3 lines is treated as the content end (the large blank before footer/header).
    let line_h = 14.0;
    let gap = 3.0 * line_h;
    let mut bottom = first;
    for w in ys.windows(2) {
        if w[1] - w[0] > gap {
            break;
        }
        bottom = w[1];
    }
    bottom + line_h
}

/// Compile typst source → PDF bytes (reuses PagedDocument; font loading shares the same whitelist as SVG).
/// No longer goes through mdxport::compile_typst_to_pdf, so the font directory is uniformly controlled by SOMADOC_FONTS_DIR.
pub fn compile_typst_to_pdf(
    source: &str,
    logo: Option<Bytes>,
    images: &[(String, Bytes)],
) -> Result<Vec<u8>, TypstError> {
    let document = compile_document(source, logo, images)?;
    let out = typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default()).map_err(|diagnostics| {
        TypstError(diagnostics.iter().map(|d| d.message.to_string()).collect())
    });
    drop(document);
    out
}

/// Compile typst source → PagedDocument (shared by PDF and SVG).
fn compile_document(
    source: &str,
    logo: Option<Bytes>,
    images: &[(String, Bytes)],
) -> Result<PagedDocument, TypstError> {
    if std::env::var("SOMADOC_TYPST_DUMP").as_deref() == Ok("1") {
        let _ = std::fs::write(std::env::temp_dir().join("typst-last-svg.typ"), source);
    }
    let world = PreviewWorld::new(source, logo, images);
    compile_document_reuse(&world)
}

/// Compile typst source → (PagedDocument, Source). The Source is used to resolve Spans in frames back to byte ranges (source map).
fn compile_document_with_source(
    source: &str,
    logo: Option<Bytes>,
    images: &[(String, Bytes)],
) -> Result<(PagedDocument, Source), TypstError> {
    if std::env::var("SOMADOC_TYPST_DUMP").as_deref() == Ok("1") {
        let _ = std::fs::write(std::env::temp_dir().join("typst-last-svg.typ"), source);
    }
    let world = PreviewWorld::new(source, logo, images);
    let document = compile_document_reuse(&world)?;
    let src = world.main_source.lock().unwrap().clone();
    Ok((document, src))
}

/// Compile with an existing persistent World (incremental preview: reuse the same World + Source::replace; comemo only re-lays-out changed parts).
fn compile_document_reuse(world: &PreviewWorld) -> Result<PagedDocument, TypstError> {
    let warned = typst::compile::<PagedDocument>(world);
    warned.output.map_err(|diagnostics| {
        TypstError(diagnostics.iter().map(|d| d.message.to_string()).collect())
    })
}

/// A unified error shared by/exposed to the PDF path (avoiding coupling to mdxport::CompileError).
#[derive(Debug)]
pub struct TypstError(pub Vec<String>);

impl std::fmt::Display for TypstError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.join("\n"))
    }
}
impl std::error::Error for TypstError {}

// ---------------------------------------------------------------------------
// World implementation (same font loading as mdxport, cached globally once)
// ---------------------------------------------------------------------------

struct PreviewWorld {
    library: LazyHash<Library>,
    main_id: FileId,
    main_source: Mutex<Source>,
    font_storage: &'static FontStore,
    /// Optional injected logo image bytes (registered at the fixed virtual path `/somadoc-logo`).
    logo: Mutex<Option<Bytes>>,
    logo_id: FileId,
    /// Inline image bytes (virtual path → bytes, e.g. `/somadoc-img/<id>`).
    images: Mutex<HashMap<String, Bytes>>,
}

impl PreviewWorld {
    fn new(source: &str, logo: Option<Bytes>, images: &[(String, Bytes)]) -> Self {
        let main_id = FileId::new(None, VirtualPath::new("/main.typ"));
        let logo_id = FileId::new(None, VirtualPath::new("/somadoc-logo"));
        let main_source = Source::new(main_id, source.to_string());
        let mut img_map = HashMap::new();
        for (k, v) in images {
            img_map.insert(k.clone(), v.clone());
        }
        Self {
            library: LazyHash::new(Library::default()),
            main_id,
            main_source: Mutex::new(main_source),
            font_storage: FontStore::global(),
            logo: Mutex::new(logo),
            logo_id,
            images: Mutex::new(img_map),
        }
    }

    /// Replace the source in place: `Source::replace` does a prefix/suffix diff + minimal-range reparse,
    /// reusing unchanged SyntaxNodes and comemo's cached eval/layout (key to incremental compilation).
    fn replace_source(&self, new: &str) {
        self.main_source.lock().unwrap().replace(new);
    }

    fn replace_logo(&self, logo: Option<Bytes>) {
        *self.logo.lock().unwrap() = logo;
    }

    fn replace_images(&self, images: &[(String, Bytes)]) {
        let mut map = self.images.lock().unwrap();
        map.clear();
        for (k, v) in images {
            map.insert(k.clone(), v.clone());
        }
    }
}

impl World for PreviewWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &self.font_storage.book
    }
    fn main(&self) -> FileId {
        self.main_id
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main_id {
            Ok(self.main_source.lock().unwrap().clone())
        } else {
            Err(FileError::NotFound(id.vpath().as_rootless_path().into()))
        }
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        {
            let logo = self.logo.lock().unwrap();
            if let Some(bytes) = logo.as_ref() {
                if id == self.logo_id {
                    return Ok(bytes.clone());
                }
            }
        }
        let path = id.vpath().as_rooted_path().to_string_lossy().into_owned();
        if let Some(bytes) = self.images.lock().unwrap().get(&path) {
            return Ok(bytes.clone());
        }
        Err(FileError::NotFound(id.vpath().as_rootless_path().into()))
    }
    fn font(&self, index: usize) -> Option<Font> {
        self.font_storage
            .fonts
            .get(index)
            .and_then(|slot| slot.get())
    }
    fn today(&self, _offset: Option<i64>) -> Option<Datetime> {
        None
    }
}

// ---------------------------------------------------------------------------
// Incremental preview: persistent World + session registry
// ---------------------------------------------------------------------------

/// Persistent compilation context for a single document session: reuses the same `PreviewWorld` (`Source` replaced in place),
/// letting typst's comemo global cache work across compilations (only changed parts are re-laid-out).
struct SessionCompiler {
    world: PreviewWorld,
    /// Serialize this session's "replace + compile" to avoid source inconsistency under concurrent edits.
    lock: Mutex<()>,
}

impl SessionCompiler {
    fn new() -> Self {
        Self {
            world: PreviewWorld::new("", None, &[]),
            lock: Mutex::new(()),
        }
    }

    /// Incrementally re-lay-out the given typst source (+ optional logo), returning per-page SVG + heading-anchor source map.
    /// Note: this path does **not** clear the font cache (fonts are cached across compilations, a typst-recommended incremental optimization).
    fn render_pages(
        &self,
        source: &str,
        logo: Option<Bytes>,
        images: &[(String, Bytes)],
        anchor_specs: &[AnchorSpec],
    ) -> Result<(Vec<String>, Vec<HeadingAnchor>), TypstError> {
        let _guard = self.lock.lock().unwrap();
        self.world.replace_source(source);
        self.world.replace_logo(logo);
        self.world.replace_images(images);
        let document = compile_document_reuse(&self.world)?;
        let src = self.world.main_source.lock().unwrap().clone();
        let anchors = extract_anchors(&document, &src, anchor_specs);
        let pages = document.pages.iter().map(typst_svg::svg).collect();
        Ok((pages, anchors))
    }
}

struct SessionEntry {
    compiler: Arc<SessionCompiler>,
    last_used: u64,
}

struct SessionRegistry {
    map: HashMap<String, SessionEntry>,
    next_id: u64,
    max: usize,
}

static SESSIONS: OnceLock<Mutex<SessionRegistry>> = OnceLock::new();

/// Get (or create) a session, evicting sessions beyond the cap in LRU order.
fn get_or_create_session(session_id: &str) -> Arc<SessionCompiler> {
    let reg = SESSIONS.get_or_init(|| {
        Mutex::new(SessionRegistry {
            map: HashMap::new(),
            next_id: 0,
            max: 64,
        })
    });
    let mut guard = reg.lock().unwrap();
    guard.next_id += 1;
    let now = guard.next_id;
    let entry = guard
        .map
        .entry(session_id.to_string())
        .or_insert_with(|| SessionEntry {
            compiler: Arc::new(SessionCompiler::new()),
            last_used: now,
        });
    entry.last_used = now;
    let compiler = entry.compiler.clone();
    if guard.map.len() > guard.max {
        let mut oldest_key: Option<String> = None;
        let mut oldest = u64::MAX;
        for (k, e) in guard.map.iter() {
            if e.last_used < oldest {
                oldest = e.last_used;
                oldest_key = Some(k.clone());
            }
        }
        if let Some(k) = oldest_key {
            guard.map.remove(&k);
        }
    }
    compiler
}

/// Incremental render: reuse a persistent World by session_id for incremental compilation, returning per-page SVG + heading-anchor source map.
/// Differs from one-shot `compile_typst_to_pages`: no World rebuild, no font-cache clear, reuses the Source.
pub fn render_pages_incremental(
    session_id: &str,
    source: &str,
    logo: Option<Bytes>,
    images: &[(String, Bytes)],
    anchor_specs: &[AnchorSpec],
) -> Result<(Vec<String>, Vec<HeadingAnchor>), TypstError> {
    get_or_create_session(session_id).render_pages(source, logo, images, anchor_specs)
}

/// Font byte source: embedded resources hold bytes directly; system fonts store only a path, read lazily on first use.
#[derive(Clone)]
enum FontSource {
    Data(Bytes),
    File(PathBuf),
}

struct FontSlot {
    source: FontSource,
    index: u32,
    font: Mutex<Option<Font>>,
}

impl FontSlot {
    fn new(source: FontSource, index: u32) -> Self {
        Self {
            source,
            index,
            font: Mutex::new(None),
        }
    }
    fn get(&self) -> Option<Font> {
        let mut guard = self.font.lock().unwrap();
        if let Some(font) = guard.as_ref() {
            return Some(font.clone());
        }
        let data = match &self.source {
            FontSource::Data(b) => b.clone(),
            FontSource::File(path) => fs::read(path).ok().map(Bytes::new)?,
        };
        let font = Font::new(data, self.index);
        *guard = font.clone();
        font
    }
    fn clear(&self) {
        let mut guard = self.font.lock().unwrap();
        *guard = None;
    }
}

struct FontStore {
    book: LazyHash<FontBook>,
    fonts: Vec<FontSlot>,
}

static FONT_STORE: OnceLock<FontStore> = OnceLock::new();

impl FontStore {
    fn global() -> &'static Self {
        FONT_STORE.get_or_init(|| {
            let mut book = FontBook::new();
            let mut fonts = Vec::new();
            for data in typst_assets::fonts() {
                let bytes = Bytes::new(data);
                add_font_data(
                    &mut book,
                    &mut fonts,
                    bytes.clone(),
                    FontSource::Data(bytes),
                );
            }
            for dir in system_font_dirs() {
                scan_font_dir(&mut book, &mut fonts, &dir);
            }
            FontStore {
                book: LazyHash::new(book),
                fonts,
            }
        })
    }

    fn clear_fonts(&self) {
        for slot in &self.fonts {
            slot.clear();
        }
    }
}

/// Explicitly release the font cache (Font objects are large). The font cache is now **resident**: reused across compilations for faster warm runs,
/// and is no longer called after every render — clearing it globally during concurrent renders caused repeated font reloads + races.
/// This function is kept for future "idle-window cleanup / background command" use.
pub fn clear_font_cache() {
    if let Some(store) = FONT_STORE.get() {
        store.clear_fonts();
    }
}

fn add_font_data(book: &mut FontBook, fonts: &mut Vec<FontSlot>, data: Bytes, source: FontSource) {
    for index in 0_u32.. {
        match FontInfo::new(data.as_slice(), index) {
            Some(info) => {
                book.push(info);
                fonts.push(FontSlot::new(source.clone(), index));
            }
            None => break,
        }
    }
}

fn scan_font_dir(book: &mut FontBook, fonts: &mut Vec<FontSlot>, dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut seen = std::collections::HashSet::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_font_dir(book, fonts, &path);
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase);
        if !matches!(ext.as_deref(), Some("ttf" | "otf" | "ttc" | "otc")) {
            continue;
        }
        if let Ok(canonical) = fs::canonicalize(&path) {
            if !seen.insert(canonical) {
                continue;
            }
        }
        // Read once to extract FontInfo (then discard); the full bytes are lazily read by FontSlot::get on first use.
        if let Ok(data) = fs::read(&path) {
            add_font_data(book, fonts, Bytes::new(data), FontSource::File(path));
        }
    }
}

fn system_font_dirs() -> Vec<PathBuf> {
    // Whitelist: when a font directory is explicitly set, only that directory is scanned (multiple dirs separated by `:`, like PATH).
    // Avoid scanning all system fonts (e.g. 2.5GB in /usr/share/fonts), which would blow up the worker's resident memory.
    if let Some(raw) = std::env::var_os("SOMADOC_FONTS_DIR") {
        let mut dirs: Vec<PathBuf> = std::env::split_paths(&raw).collect();
        dirs.retain(|p| !p.as_os_str().is_empty());
        if !dirs.is_empty() {
            return dirs;
        }
    }
    let mut dirs = Vec::new();
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/System/Library/Fonts"));
        dirs.push(PathBuf::from("/Library/Fonts"));
        if let Some(h) = home_dir() {
            dirs.push(h.join("Library/Fonts"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        dirs.push(PathBuf::from("/usr/share/fonts"));
        dirs.push(PathBuf::from("/usr/local/share/fonts"));
        if let Some(h) = home_dir() {
            dirs.push(h.join(".local/share/fonts"));
            dirs.push(h.join(".fonts"));
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(w) = std::env::var_os("WINDIR") {
            dirs.push(PathBuf::from(w).join("Fonts"));
        }
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(l).join("Microsoft\\Fonts"));
        }
    }
    if let Some(h) = home_dir() {
        dirs.push(h.join(".mdxport").join("fonts"));
    }
    dirs
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}
