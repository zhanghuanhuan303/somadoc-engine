// SomaDoc general-purpose template — Typst version (template.typ).
// A close visual match to the classic pandoc + xelatex article look (calibrated
// against rendered output):
//   A4 · 1in margins · IBM Plex Sans SC 12pt · leading ≈ 14.35pt · paragraph spacing 20pt
//   headings: h1 17.28pt centered / h2 14.4pt left / h3 12pt left (titlespacing before/after)
//   lists: unordered marker = ・ (CJK middle dot) · quotes = tinted fill + left rule + italic
//          · tables = booktabs three-line
//   math: inline and block (handwritten LaTeX is converted to Typst upstream)
//   page header = current level-2 section name · footer = centered page number
// Contract: article(title, authors, lang, toc, body)

// Remember the most recent level-2 section name for the running header.
#let _secname = state("soma-sec", none)
#let _v(key, default: "") = _somadoc_vars.at(key, default: default)
#let _lf(key, default) = { let s = _v(key); if s == "" { default } else { float(s) } }

#let article(
  title: none,
  authors: (),
  lang: "zh",
  toc: false,
  body,
) = {
  set page(
    paper: "a4",
    margin: (top: 1in, bottom: 1in, x: 1in),
    // Centered page number in the footer.
    footer: context align(center + horizon, counter(page).display()),
    // Running header: the rule hugs the bottom of the section name. Drawing it
    // with a `box` bottom stroke (and wrapping the text in that box) keeps the
    // line tight under the text and avoids the `show line` rule below, which
    // only handles horizontal rules in the body.
    header: context [
      #if counter(page).get().first() != 1 {
        let sec = _secname.get()
        let _hlogo = _v("logo") != ""
        if _hlogo or (sec != none and sec != "") {
          box(
            width: 100%,
            stroke: (bottom: 0.5pt + luma(150)),
            inset: (bottom: 2pt),
            grid(
              columns: (1fr, auto),
              column-gutter: 1em,
              align(left + bottom, if _hlogo {
                image("/somadoc-logo", height: 1em)
              } else {
                []
              }),
              align(right + bottom, if sec != none and sec != "" {
                text(size: 9pt, fill: luma(120))[#sec]
              } else {
                []
              }),
            ),
          )
        }
      }
    ],
  )
  // Hyphenation dictionary: Typst loads it based on `lang`. A `zh` document has
  // no English dictionary, so mixed-in English words would not be hyphenated
  // (poor typography). Using the `en` dictionary when the body is Chinese lets
  // English words hyphenate; other languages keep their own `lang`. CJK line
  // breaking does not depend on `lang`, so there is no side effect.
  let _h_lang = if lang.starts-with("zh") or lang == "" { "en" } else { lang }
  set text(font: (
    if _v("mainfont") != "" { _v("mainfont") } else { "IBM Plex Sans SC" },
    "Noto Sans CJK SC",
    "PingFang SC",
    "Microsoft YaHei",
  ), size: 12pt, lang: _h_lang, hyphenate: true) // hyphenate at line ends to improve mixed CJK/Latin text
  // Line spacing: body baseline ≈ 15.8pt (leading plus the font's default line
  // height) — slightly airier than the 14.35pt of the original.
  set par(justify: true, leading: 7pt, spacing: 28pt)
  // Footnotes: a short thin rule between body and footnotes (30% of the text
  // width, 0.4pt), with the footnote text automatically scaled down.
  set footnote(numbering: "1") // arabic superscript numerals
  set footnote.entry(
    separator: line(length: 30%, stroke: 0.4pt + luma(120)),
    clearance: 0.7em, // gap between body and separator
    gap: 0.4em, // gap between footnote entries
    indent: 0.5em, // first-line indent after the footnote number
  )
  // Thematic break (`---`): centered. The engine rewrites the emitted full-width
  // rule to a half-width one, mirroring the original 0.5-linewidth rule.
  show line: it => align(center, it)

  // Headings: size and alignment, plus recording the level-2 section name.
  show heading: it => {
    if it.level == 2 { _secname.update(it.body) }
    it
  }
  set heading(numbering: none, supplement: none)
  // h1 is centered; h2/h3 are left-aligned. Vertical spacing is generous (at
  // least two character heights below each heading).
  show heading.where(level: 1): it => block(above: 2.5em, below: 1.8em, width: 100% + 0pt, align(center, text(size: 17.3pt, weight: "bold", it)))
  show heading.where(level: 2): it => block(above: 2em, below: 1.8em, align(left, text(size: 14.4pt, weight: "bold", it)))
  show heading.where(level: 3): it => block(above: 1.8em, below: 1.6em, align(left, text(size: 12pt, weight: "bold", it)))

  // Lists: `・` marker when unordered, numerals when ordered; clearly indented
  // with roomy item spacing and at least two character heights above and below.
  show list: set block(above: 28pt, below: 28pt)
  show list: set list(marker: "・", indent: 32pt, body-indent: 14pt, spacing: 0.9em)
  show enum: set block(above: 28pt, below: 28pt)
  show enum: set enum(numbering: "1.", indent: 32pt, body-indent: 14pt, spacing: 0.9em)

  // Quotes: tinted fill, left rule and italic text, with slightly looser leading.
  show quote: it => block(
    above: 28pt,
    below: 28pt,
    fill: luma(245),
    stroke: (left: 0.8pt + rgb("#9E9E9E"), top: 0.1pt + luma(230), bottom: 0.1pt + luma(230), right: 0.1pt + luma(230)),
    inset: (x: 1.2em, y: 0.6em),
    width: 100%,
  )[
    #set text(style: "italic")
    #set par(leading: 8pt)
    #it.body
  ]

  // Tables: booktabs style — heavy top rule, thin rule under the header, no
  // vertical rules. The horizontal rules are drawn by a `stroke` function
  // (header = row 0); at least two character heights above and below.
  show table: set block(above: 28pt, below: 28pt)
  show table: set align(center)
  show table: set table(
    inset: (x: 10pt, y: 6pt),
    align: center + horizon,
    stroke: (x, y) => (
      top: if y == 0 { 1pt + luma(60) } else if y == 1 { 0.5pt + luma(120) } else { 0pt },
      left: none,
      right: none,
    ),
  )
  show table.header: strong

  // Inline code and code blocks: monospace.
  show raw.where(block: false): set text(font: ("JetBrains Mono", "DejaVu Sans Mono", "IBM Plex Mono"), size: 10.5pt)
  show raw.where(block: true): block.with(above: 28pt, below: 28pt, fill: luma(244), inset: (x: 10pt, y: 6pt))
  show raw.where(block: true): set text(font: ("JetBrains Mono", "DejaVu Sans Mono", "IBM Plex Mono"), size: 10pt)

  show link: set text(fill: rgb("#1E88E5"))

  // Title block: a large centered title followed by an author line.
  if _v("logo") != "" {
    align(center, image("/somadoc-logo", height: 2.5em))
    v(1.2em)
  }
  if title != none and title != "" {
    align(center, text(size: 17.3pt, weight: "bold")[#title])
    v(1em)
  }
  if authors != () {
    // `authors` is a tuple; `#()` evaluates the `.join(", ")` call (otherwise it
    // would be taken literally inside a content block).
    align(center, text(size: 12pt)[#(authors.join(", "))])
    v(1em)
  }

  body
}
