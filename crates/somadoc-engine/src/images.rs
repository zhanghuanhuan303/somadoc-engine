//! Inline-image translation: Markdown → typst `image`.
//!
//! Uses the same "placeholder protection" pattern as `latex_commands.rs`:
//!   1. `protect_images(md, images)` — scan Markdown for `![alt](/somadoc-img/<id>)`,
//!      replace it with an alphanumeric placeholder, and collect (placeholder → typst snippet)
//!      plus the image bytes (virtual path → `Bytes`).
//!   2. `mdxport::convert` — placeholders pass through as plain text (untouched by comrak).
//!   3. `restore_images(body, replacements)` — restore placeholders into typst image snippets.
//!
//! Layout mapping (`InlineImage.align`):
//!   center (default) → `#align(center, image(...))`
//!   left / right → `#align(left|right, image(...))`
//!   wrap-left / wrap-right → `#figure(image(...), placement: left|right)` (text wrap)

use std::collections::HashMap;

use crate::types::InlineImage;
use base64::Engine;
use typst::foundations::Bytes;

/// Inline image bytes (virtual path → bytes).
pub type ImageBytes = Vec<(String, Bytes)>;

const IMG_PREFIX: &str = "SOMADOCIMG";
const IMG_SUFFIX: &str = "K2M8X";

/// Image injection result: placeholder → typst snippet; virtual path → image bytes.
pub struct ImageInject {
    pub replacements: HashMap<String, String>,
    pub bytes: Vec<(String, Bytes)>,
}

/// Layout → typst snippet template (`{path}` = virtual path, `{alt}` = escaped alt text).
fn layout_template(align: &str) -> &'static str {
    match align {
        "left" => "#align(left, image(\"{path}\", alt: \"{alt}\"))",
        "right" => "#align(right, image(\"{path}\", alt: \"{alt}\"))",
        "wrap-left" => "#figure(image(\"{path}\", alt: \"{alt}\"), placement: left)",
        "wrap-right" => "#figure(image(\"{path}\", alt: \"{alt}\"), placement: right)",
        _ => "#align(center, image(\"{path}\", alt: \"{alt}\"))",
    }
}

/// Scan the Markdown body and replace `![alt](/somadoc-img/<id>)` with a placeholder.
pub fn protect_images(markdown: &str, images: &[InlineImage]) -> (String, ImageInject) {
    let by_id: HashMap<&str, &InlineImage> = images.iter().map(|i| (i.id.as_str(), i)).collect();

    let chars: Vec<char> = markdown.chars().collect();
    let mut out = String::with_capacity(markdown.len());
    let mut inject = ImageInject {
        replacements: HashMap::new(),
        bytes: Vec::new(),
    };
    let mut counter = 0usize;

    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '!' && i + 1 < chars.len() && chars[i + 1] == '[' {
            if let Some((alt, url, end)) = scan_md_image(&chars, i) {
                if let Some(id) = url.strip_prefix("/somadoc-img/") {
                    if let Some(img) = by_id.get(id) {
                        if let Ok(data) =
                            base64::engine::general_purpose::STANDARD.decode(img.data.as_bytes())
                        {
                            let vpath = format!("/somadoc-img/{id}");
                            let placeholder = format!("{IMG_PREFIX}{counter}{IMG_SUFFIX}");
                            counter += 1;
                            let align = img.align.as_deref().unwrap_or("center");
                            let typst = layout_template(align)
                                .replace("{path}", &vpath)
                                .replace("{alt}", &escape_typst_string(&alt));
                            inject.replacements.insert(placeholder.clone(), typst);
                            inject.bytes.push((vpath, Bytes::new(data)));
                            out.push_str(&placeholder);
                            i = end;
                            continue;
                        }
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    (out, inject)
}

/// Restore image placeholders in the typst body into typst snippets.
pub fn restore_images(body: &str, replacements: &HashMap<String, String>) -> String {
    if replacements.is_empty() {
        return body.to_string();
    }
    let mut out = body.to_string();
    for (placeholder, typst) in replacements {
        out = out.replace(placeholder.as_str(), typst.as_str());
    }
    out
}

/// Scan `![alt](url)` (chars[start]=='!' and chars[start+1]=='['); returns (alt, url, end index).
/// Neither alt nor url supports nested parens/brackets (sufficient for v1); returns `None` on failure.
fn scan_md_image(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    let mut j = start + 2;
    let mut alt = String::new();
    while j < chars.len() && chars[j] != ']' {
        alt.push(chars[j]);
        j += 1;
    }
    if j >= chars.len() || j + 1 >= chars.len() || chars[j + 1] != '(' {
        return None;
    }
    let mut k = j + 2;
    let mut url = String::new();
    while k < chars.len() && chars[k] != ')' {
        url.push(chars[k]);
        k += 1;
    }
    if k >= chars.len() {
        return None;
    }
    Some((alt, url, k + 1))
}

/// Escape into a typst double-quoted string literal.
fn escape_typst_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(id: &str, align: &str) -> InlineImage {
        InlineImage {
            id: id.to_string(),
            data: base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n"),
            align: Some(align.to_string()),
        }
    }

    #[test]
    fn rewrites_image_to_center_by_default() {
        let md = "前文\n\n![示意](/somadoc-img/a1)\n\n后文";
        let (protected, inject) = protect_images(md, &[img("a1", "center")]);
        assert!(!protected.contains("![示意]"));
        assert!(protected.contains("SOMADOCIMG0K2M8X"));
        let restored = restore_images(&protected, &inject.replacements);
        assert!(restored.contains("#align(center, image(\"/somadoc-img/a1\", alt: \"示意\"))"));
        assert_eq!(inject.bytes.len(), 1);
        assert_eq!(inject.bytes[0].0, "/somadoc-img/a1");
    }

    #[test]
    fn leaves_ordinary_images_as_is() {
        let md = "![外部](https://example.com/x.png)";
        let (protected, inject) = protect_images(md, &[]);
        assert_eq!(protected, md);
        assert!(inject.replacements.is_empty());
    }
}
