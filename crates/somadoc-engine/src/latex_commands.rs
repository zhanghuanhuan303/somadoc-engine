//! Handwritten LaTeX commands → Typst translation layer.
//!
//! Background: mdxport/comrak only recognize `$…$` / `$$…$$` math; handwritten LaTeX
//! commands in a document (e.g. `\textbf{…}`, `\color{#hex}{…}`, `\normalsize`) get
//! escaped by `escape_text` into literal text and cannot render. This module inserts
//! "placeholder protection" into the `markdown → typst` pipeline:
//!
//! 1. `protect_markdown(md)` — scan raw LaTeX command groups, recursively translate
//!    them to Typst, and replace them with alphanumeric placeholders (which survive
//!    comrak's escaping).
//! 2. `mdxport::convert(...)` — convert normally (placeholders pass through as text).
//! 3. `restore_placeholders(body)` — replace placeholders back with real Typst snippets.
//!
//! Math spans (`$…$` / `$$…$$`) are copied through untouched: mdxport converts LaTeX math
//! itself, and its math converter separates identifier characters with spaces, so a
//! placeholder inserted inside math could never be matched again by `restore_placeholders`.
//!
//! `named_to_typst` / `parse_control_symbol` are the authoritative command list,
//! organized by category; update their unit tests when adding a command.

use std::collections::HashMap;

const PLACEHOLDER_PREFIX: &str = "SOMADOCLATEXCMD";
const PLACEHOLDER_SUFFIX: &str = "Q7F3A";

/// Scan raw LaTeX commands in markdown and replace them with placeholders.
///
/// Returns `(protected markdown, placeholder → Typst snippet)`.
/// Placeholders are alphanumeric, so they pass through comrak's `escape_text` untouched.
pub fn protect_markdown(markdown: &str) -> (String, HashMap<String, String>) {
    let mut translator = LatexTranslator::new();
    let mut out = String::with_capacity(markdown.len() + 32);
    translator.scan_markdown(markdown, &mut out);
    (out, translator.replacements)
}

/// Restore placeholders in the typst body into Typst snippets.
pub fn restore_placeholders(body: &str, replacements: &HashMap<String, String>) -> String {
    if replacements.is_empty() {
        return body.to_string();
    }
    let mut out = body.to_string();
    for (placeholder, typst) in replacements {
        out = out.replace(placeholder.as_str(), typst.as_str());
    }
    out
}

struct LatexTranslator {
    counter: usize,
    replacements: HashMap<String, String>,
}

impl LatexTranslator {
    fn new() -> Self {
        Self {
            counter: 0,
            replacements: HashMap::new(),
        }
    }

    fn alloc_placeholder(&mut self) -> String {
        let ph = format!("{PLACEHOLDER_PREFIX}{}{PLACEHOLDER_SUFFIX}", self.counter);
        self.counter += 1;
        ph
    }

    /// Top-level scan: replace raw LaTeX commands with placeholders; other characters pass through (comrak handles the markdown syntax).
    /// Text-style commands (bold/italic/underline/strike/super/sub/mono) have Markdown equivalents and are rewritten
    /// to Markdown (inner content is scanned recursively, keeping nested `**`/`*`/` `` `), so nested Markdown renders correctly.
    fn scan_markdown(&mut self, input: &str, out: &mut String) {
        let chars: Vec<char> = input.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' {
                // `\$` is an escaped dollar, not a math delimiter: keep it verbatim so the
                // `$` below is not mistaken for the start of a math span.
                if chars.get(i + 1) == Some(&'$') {
                    out.push('\\');
                    out.push('$');
                    i += 2;
                    continue;
                }
                // 1. text style → Markdown rewrite (keep nested markdown)
                if let Some((md, next)) = self.parse_markdown_wrap(&chars, i) {
                    out.push_str(&md);
                    i = next;
                    continue;
                }
                // 2. other commands → placeholder + Typst
                if let Some((typst, next)) = self.parse_command(&chars, i) {
                    let placeholder = self.alloc_placeholder();
                    self.replacements.insert(placeholder.clone(), typst);
                    out.push_str(&placeholder);
                    i = next;
                    continue;
                }
                // unknown command → keep the backslash as a literal (comrak treats it as text)
                out.push('\\');
                i += 1;
            } else if chars[i] == '$' {
                // Math spans are copied verbatim: mdxport converts LaTeX math to Typst math
                // itself, so rewriting commands inside math only corrupts it.
                match math_span(&chars, i) {
                    Some(next) => {
                        out.extend(&chars[i..next]);
                        i = next;
                    }
                    // an unterminated `$` is not a math span
                    None => {
                        out.push('$');
                        i += 1;
                    }
                }
            } else {
                out.push(chars[i]);
                i += 1;
            }
        }
    }

    /// Text-style command (with a Markdown equivalent) → rewritten as a Markdown wrap.
    /// Inner content is scanned recursively via `scan_markdown`: nested LaTeX commands become placeholders, Markdown markers are kept.
    fn parse_markdown_wrap(&mut self, chars: &[char], i: usize) -> Option<(String, usize)> {
        let mut j = i + 1;
        let name_start = j;
        while j < chars.len() && chars[j].is_ascii_alphabetic() {
            j += 1;
        }
        let name: String = chars[name_start..j].iter().collect();
        let (open, close) = markdown_wrap(&name)?;
        let (content, next) = self.parse_braced_group(chars, j)?;
        let mut inner = String::new();
        self.scan_markdown(&content, &mut inner);
        Some((format!("{open}{inner}{close}"), next))
    }

    /// Recursively translate a command-argument string to Typst (nested commands translate too; plain text is Typst-escaped).
    fn content_to_typst(&mut self, content: &str) -> String {
        let chars: Vec<char> = content.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' {
                if let Some((typst, next)) = self.parse_command(&chars, i) {
                    out.push_str(&typst);
                    i = next;
                } else {
                    // unknown command → literal backslash
                    out.push_str("\\\\");
                    i += 1;
                }
            } else {
                out.push_str(&escape_typst_char(chars[i]));
                i += 1;
            }
        }
        out
    }

    /// Recursively translate a command-argument string to Typst, treating it as Markdown (nested LaTeX → placeholder, Markdown → Typst).
    /// Used for commands like `\underline` that need to control the Typst wrapper themselves (e.g. `evade: false`).
    fn md_content_to_typst(&mut self, content: &str) -> String {
        // 1. first recursively replace nested LaTeX with placeholders (Markdown syntax is kept)
        let mut marked = String::new();
        self.scan_markdown(content, &mut marked);
        // 2. use mdxport to convert the remaining Markdown to Typst (alphanumeric placeholders pass through)
        md_to_typst(&marked)
    }

    /// Parse a LaTeX command at `chars[i]` (a `\`).
    /// Returns `(typst snippet, next unconsumed index)` on success; `None` on failure (caller keeps the `\`).
    fn parse_command(&mut self, chars: &[char], i: usize) -> Option<(String, usize)> {
        debug_assert_eq!(chars[i], '\\');
        let mut j = i + 1;
        let name_start = j;
        while j < chars.len() && chars[j].is_ascii_alphabetic() {
            j += 1;
        }
        let name: String = chars[name_start..j].iter().collect();
        if name.is_empty() {
            self.parse_control_symbol(chars, i)
        } else {
            self.parse_named_command(&name, chars, j)
        }
    }

    fn parse_named_command(
        &mut self,
        name: &str,
        chars: &[char],
        j: usize,
    ) -> Option<(String, usize)> {
        // Declarative size: \large may be followed by {…}; without {…} it is a declaration (treated as a no-op: just drop the command).
        if let Some(size) = size_pt(name) {
            let mut idx = j;
            while idx < chars.len() && chars[idx].is_whitespace() {
                idx += 1;
            }
            if idx < chars.len() && chars[idx] == '{' {
                let (arg, next) = self.parse_braced_group(chars, idx)?;
                let content = self.content_to_typst(&arg);
                return Some((format!("#text(size: {size})[{content}]"), next));
            }
            // no braces: don't consume trailing whitespace; return the original position
            return Some((String::new(), j));
        }

        let arity = command_arity(name)?;
        let mut idx = j;
        let mut raw_args = Vec::with_capacity(arity);
        for _ in 0..arity {
            let (arg, next) = self.parse_braced_group(chars, idx)?;
            raw_args.push(arg);
            idx = next;
        }
        let typst = self.named_to_typst(name, &raw_args)?;
        Some((typst, idx))
    }

    fn parse_control_symbol(&mut self, chars: &[char], i: usize) -> Option<(String, usize)> {
        if i + 1 >= chars.len() {
            return None;
        }
        let (typst, next) = match chars[i + 1] {
            '\\' => ("#linebreak()".to_string(), i + 2),
            ' ' => (" ".to_string(), i + 2),
            ',' => ("#h(0.1667em)".to_string(), i + 2),
            ';' => ("#h(0.2778em)".to_string(), i + 2),
            ':' => ("#h(0.2222em)".to_string(), i + 2),
            '!' => ("#h(-0.1667em)".to_string(), i + 2),
            _ => return None,
        };
        Some((typst, next))
    }

    /// Parse a braced group `{ … }` (nested groups supported); returns `(inner content, next index)`.
    fn parse_braced_group(&self, chars: &[char], i: usize) -> Option<(String, usize)> {
        let mut idx = i;
        while idx < chars.len() && chars[idx].is_whitespace() {
            idx += 1;
        }
        if idx >= chars.len() || chars[idx] != '{' {
            return None;
        }
        let mut depth = 0usize;
        let mut j = idx;
        let mut content = String::new();
        while j < chars.len() {
            match chars[j] {
                '{' => {
                    if depth > 0 {
                        content.push('{');
                    }
                    depth += 1;
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((content, j + 1));
                    }
                    content.push('}');
                }
                c => content.push(c),
            }
            j += 1;
        }
        None
    }

    /// Named command → Typst. Organized by category (this function is the "executable list").
    fn named_to_typst(&mut self, name: &str, args: &[String]) -> Option<String> {
        let typst = match name {
            // ── F. spacing / line break / page break (0 args) ─────────
            "quad" => "#h(1em)".to_string(),
            "qquad" => "#h(2em)".to_string(),
            "enspace" | "enskip" => "#h(0.5em)".to_string(),
            "hfill" => "#h(1fr)".to_string(),
            "newline" => "#linebreak()".to_string(),
            "newpage" | "pagebreak" | "clearpage" => "#pagebreak()".to_string(),

            // ── A. text style (1 arg, content) ────────────────────────
            // Note: bold/italic/strike/super/sub/mono/smallcaps are rewritten to Markdown at the top level by `markdown_wrap` (keeping nesting);
            // here we only handle the "nested inside a placeholder command" case, using arg_content (escaped).
            // underline is special: Typst's default evade=true breaks the underline around glyphs; evade:false keeps it continuous.
            "textbf" | "bf" => format!("#strong[{}]", self.arg_content(args, 0)?),
            "textit" | "it" | "emph" => format!("#emph[{}]", self.arg_content(args, 0)?),
            "underline" => format!(
                "#underline(evade: false)[{}]",
                self.md_content_to_typst(args.first()?)
            ),
            "textsc" => format!("#smallcaps[{}]", self.arg_content(args, 0)?),
            "textsuperscript" => format!("#super[{}]", self.arg_content(args, 0)?),
            "textsubscript" => format!("#sub[{}]", self.arg_content(args, 0)?),
            "sout" | "strikeout" => format!("#strike[{}]", self.arg_content(args, 0)?),
            "texttt" => format!("#raw[{}]", self.arg_content(args, 0)?),
            // font family commands: Typst has no generic family keyword; the default font is already sans → keep content (drop the command)
            "textsf" | "textrm" | "textnormal" | "rm" => self.arg_content(args, 0)?,

            // ── C. color (2 args: color + content) ────────────────────
            "textcolor" | "color" => format!(
                "#text(fill: {})[{}]",
                typst_color(args.first()?),
                self.arg_content(args, 1)?
            ),
            "colorbox" => format!(
                "#highlight(fill: {})[{}]",
                typst_color(args.first()?),
                self.arg_content(args, 1)?
            ),

            // ── D. math font / accents (1 arg, best-effort) ───────────
            // Use a content block `[...]` rather than parens, to avoid a nested `#text(...)` in the argument triggering "# is not allowed" in code mode.
            "mathsf" => format!("#math.sans[{}]", self.arg_content(args, 0)?),
            "mathbf" => format!("#math.bold[{}]", self.arg_content(args, 0)?),
            "mathrm" => format!("#math.upright[{}]", self.arg_content(args, 0)?),
            "mathit" => format!("#math.italic[{}]", self.arg_content(args, 0)?),
            "mathtt" => format!("#math.mono[{}]", self.arg_content(args, 0)?),
            "mathcal" => format!("#math.cal[{}]", self.arg_content(args, 0)?),
            "mathfrak" => format!("#math.frak[{}]", self.arg_content(args, 0)?),
            "mathbb" => format!("#math.bb[{}]", self.arg_content(args, 0)?),
            "boldsymbol" => format!("#math.bold[{}]", self.arg_content(args, 0)?),
            "overline" => format!("#overline[{}]", self.arg_content(args, 0)?),
            // Typst has no wavy underline; approximate with an underline for now
            "utilde" => format!("#underline[{}]", self.arg_content(args, 0)?),

            // ── G. links (2 args / 1 arg) ────────────────────────────
            "href" => format!(
                "#link({})[{}]",
                typst_string(args.first()?),
                self.arg_content(args, 1)?
            ),
            "url" => format!(
                "#link({})[{}]",
                typst_string(args.first()?),
                typst_string(args.first()?)
            ),

            _ => return None,
        };
        Some(typst)
    }

    /// Take the i-th argument and recursively translate its content to Typst.
    fn arg_content(&mut self, args: &[String], i: usize) -> Option<String> {
        Some(self.content_to_typst(args.get(i)?))
    }
}

/// If `chars[start]` opens a math span, return the index just past its closing delimiter
/// (`$…$` or `$$…$$`); otherwise `None` (an unterminated `$` is literal text).
///
/// Math content is deliberately left untouched. `mdxport` converts LaTeX math to Typst math
/// itself, and rewriting commands inside math would corrupt it: its math converter separates
/// identifier characters with spaces, so a placeholder inserted here could never be restored.
fn math_span(chars: &[char], start: usize) -> Option<usize> {
    debug_assert_eq!(chars[start], '$');
    let doubled = chars.get(start + 1) == Some(&'$');
    let mut j = start + if doubled { 2 } else { 1 };
    while j < chars.len() {
        if chars[j] == '$' {
            if !doubled {
                return Some(j + 1);
            }
            if chars.get(j + 1) == Some(&'$') {
                return Some(j + 2);
            }
        }
        j += 1;
    }
    None
}

/// Command arity (0/1/2). Returns `None` for an unknown command.
/// Size commands are not in this table (they get special handling for an optional braced arg in `parse_named_command`).
fn command_arity(name: &str) -> Option<usize> {
    match name {
        // 0 args: spacing / line break / page break
        "quad" | "qquad" | "enspace" | "enskip" | "hfill" | "newline" | "newpage" | "pagebreak"
        | "clearpage" => Some(0),
        // 1 arg: text style / math font / link
        "textbf" | "bf" | "textit" | "it" | "emph" | "underline" | "textsc" | "textsuperscript"
        | "textsubscript" | "sout" | "strikeout" | "texttt" | "textsf" | "textrm"
        | "textnormal" | "rm" | "mathsf" | "mathbf" | "mathrm" | "mathit" | "mathtt"
        | "mathcal" | "mathfrak" | "mathbb" | "boldsymbol" | "overline" | "utilde" | "url" => {
            Some(1)
        }
        // 2 args: color / link
        "textcolor" | "color" | "colorbox" | "href" => Some(2),
        _ => None,
    }
}

/// Text-style command (with a Markdown equivalent) → rewritten as a Markdown wrap pair.
/// Returns `None` if not this kind of command. Inner content stays Markdown, letting comrak handle nesting.
fn markdown_wrap(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        "textbf" | "bf" => Some(("**", "**")),
        "textit" | "it" | "emph" => Some(("*", "*")),
        "sout" | "strikeout" => Some(("~~", "~~")),
        "textsuperscript" => Some(("^", "^")),
        "textsubscript" => Some(("~", "~")),
        "texttt" => Some(("`", "`")),
        _ => None,
    }
}

/// Declarative size → Typst `size` value. Returns `None` if not a size command.
fn size_pt(name: &str) -> Option<&'static str> {
    match name {
        "tiny" => Some("5pt"),
        "scriptsize" => Some("7pt"),
        "footnotesize" => Some("8pt"),
        "small" => Some("9pt"),
        "normalsize" => Some("10pt"),
        "large" => Some("12pt"),
        "Large" => Some("14.4pt"),
        "LARGE" => Some("17.28pt"),
        "huge" => Some("20.74pt"),
        "Huge" => Some("24.88pt"),
        _ => None,
    }
}

/// Use mdxport to convert a Markdown snippet to a Typst body (returns it unchanged on failure).
fn md_to_typst(content: &str) -> String {
    use mdxport::{convert, frontmatter};
    convert::convert_markdown_to_typst(
        content,
        &frontmatter::FrontMatter::default(),
        &convert::ConvertOptions::default(),
    )
    .map(|doc| doc.body.trim().to_string())
    .unwrap_or_else(|_| content.to_string())
}

/// LaTeX color name / `#hex` → Typst color expression.
fn typst_color(spec: &str) -> String {
    let s = spec.trim();
    if s.starts_with('#') {
        format!("rgb(\"{s}\")")
    } else {
        s.to_string()
    }
}

/// Convert into a Typst double-quoted string literal (for `#link("url")`).
fn typst_string(input: &str) -> String {
    format!("\"{}\"", input.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Single character → Typst text escape (matching mdxport's `escape_text`; `~` stays a non-breaking space).
fn escape_typst_char(c: char) -> String {
    match c {
        '\\' => "\\\\".into(),
        '#' => "\\#".into(),
        '[' => "\\[".into(),
        ']' => "\\]".into(),
        '{' => "\\{".into(),
        '}' => "\\}".into(),
        '*' => "\\*".into(),
        '_' => "\\_".into(),
        '$' => "\\$".into(),
        '`' => "\\`".into(),
        '@' => "\\@".into(),
        '<' => "\\<".into(),
        '>' => "\\>".into(),
        c => c.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translate(md: &str) -> String {
        let (protected, map) = protect_markdown(md);
        // Simulate mdxport: placeholders stay as literal text (we use the protected text as the "converted body").
        restore_placeholders(&protected, &map)
    }

    #[test]
    fn text_style_commands() {
        assert_eq!(translate(r"\textbf{你好}"), "**你好**");
        assert_eq!(translate(r"\textit{x}"), "*x*");
        assert_eq!(translate(r"\underline{x}"), "#underline(evade: false)[x]");
        assert_eq!(translate(r"\textsc{abc}"), "#smallcaps[abc]");
        assert_eq!(translate(r"\texttt{code}"), "`code`");
        assert_eq!(translate(r"\sout{old}"), "~~old~~");
    }

    #[test]
    fn nested_markdown_inside_text_style() {
        assert_eq!(
            translate(r"\underline{例如**强调一段同时含 *斜体* 与`代码`的句子**。}"),
            "#underline(evade: false)[例如*强调一段同时含 _斜体_ 与`代码`的句子*。]"
        );
    }

    #[test]
    fn color_commands() {
        assert_eq!(
            translate(r"\textcolor{red}{注意}"),
            "#text(fill: red)[注意]"
        );
        assert_eq!(
            translate(r"\color{#ff70b7}{pink}"),
            "#text(fill: rgb(\"#ff70b7\"))[pink]"
        );
    }

    #[test]
    fn size_commands() {
        assert_eq!(translate(r"\large{大}"), "#text(size: 12pt)[大]");
        assert_eq!(translate(r"\Huge{巨}"), "#text(size: 24.88pt)[巨]");
        // declarative (no braces) → no-op
        assert_eq!(translate(r"a\normalsize b"), "a b");
    }

    #[test]
    fn spacing_and_break_commands() {
        assert_eq!(translate(r"\quad"), "#h(1em)");
        assert_eq!(translate(r"\qquad"), "#h(2em)");
        assert_eq!(translate(r"\,"), "#h(0.1667em)");
        assert_eq!(translate(r"a\\b"), "a#linebreak()b");
        assert_eq!(translate(r"\newpage"), "#pagebreak()");
    }

    #[test]
    fn math_font_commands() {
        assert_eq!(translate(r"\mathsf{A}"), "#math.sans[A]");
        assert_eq!(translate(r"\mathbb{R}"), "#math.bb[R]");
        assert_eq!(translate(r"\utilde{x}"), "#underline[x]");
    }

    #[test]
    fn nested_commands() {
        assert_eq!(
            translate(r"\color{red}{\textbf{hi}}"),
            "#text(fill: red)[#strong[hi]]"
        );
    }

    #[test]
    fn non_breaking_space_preserved_inside_command() {
        assert_eq!(
            translate(r"\color{#f}{a~~b}"),
            "#text(fill: rgb(\"#f\"))[a~~b]"
        );
    }

    #[test]
    fn control_space_becomes_regular_space() {
        assert_eq!(translate(r"a\ b"), "a b");
    }

    #[test]
    fn unknown_command_preserved_as_literal() {
        let out = translate(r"\foo{bar}");
        assert!(
            out.contains("\\foo"),
            "unknown command should stay literal: {out}"
        );
    }

    #[test]
    fn plain_markdown_untouched() {
        assert_eq!(
            translate("# 标题\n\n**加粗** 和 `代码`"),
            "# 标题\n\n**加粗** 和 `代码`"
        );
    }

    #[test]
    fn latex_inside_math_is_left_untouched() {
        // Regression: rewriting `\,` inside math produced a placeholder that mdxport's math
        // converter could not round-trip, so it leaked into the rendered equation as
        // "SOMADOCLATEXCMD…". Math spans are now copied through verbatim.
        assert_eq!(translate(r"$e^{-x^2}\,dx$"), r"$e^{-x^2}\,dx$");
        assert_eq!(
            translate("$$\n\\int_0^\\infty e^{-x^2}\\,dx = \\frac{\\sqrt{\\pi}}{2}\n$$"),
            "$$\n\\int_0^\\infty e^{-x^2}\\,dx = \\frac{\\sqrt{\\pi}}{2}\n$$"
        );
    }

    #[test]
    fn commands_outside_math_are_still_translated() {
        assert_eq!(
            translate(r"$\alpha$ 与 \textcolor{red}{红}"),
            r"$\alpha$ 与 #text(fill: red)[红]"
        );
    }

    #[test]
    fn escaped_dollar_is_not_a_math_delimiter() {
        assert_eq!(translate(r"\$5 and \textbf{bold}"), r"\$5 and **bold**");
    }

    #[test]
    fn unterminated_dollar_is_literal() {
        assert_eq!(
            translate(r"price $5 and \textbf{bold}"),
            r"price $5 and **bold**"
        );
    }

    #[test]
    fn full_example() {
        let out =
            translate(r"\mathsf{\color{#ff70b7}{\normalsize  \utilde{~~~@2025~~linnon~lab~~~}}}");
        assert!(out.contains("rgb(\"#ff70b7\")"), "{out}");
        assert!(out.contains("underline"), "{out}");
    }
}
