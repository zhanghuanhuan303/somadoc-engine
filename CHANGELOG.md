# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Nothing yet.

## [0.1.0] - 2026-09-20

### Added

- Initial public release of the `somadoc-core` and `somadoc-engine` crates.
- Markdown → Typst → PDF rendering, compiled in-process with no `pandoc`, LaTeX distribution, or
  subprocess.
- Per-page SVG rendering with a block-level source map (anchors) for incremental preview and
  editor ↔ preview positioning.
- Handwritten LaTeX command translation to Typst, including text styles, colors, sizes, spacing, and
  math fonts.
- Booktabs three-line tables and footnotes.
- CJK text support, with font discovery whitelisted through `SOMADOC_FONTS_DIR`.
- Custom Typst templates at `<template-root>/<slug>/template.typ` (the reference template ships at
  `crates/somadoc-engine/templates/soma/template.typ`), with YAML frontmatter variables passed
  through as the `_somadoc_vars` dictionary.

[Unreleased]: https://github.com/zhanghuanhuan303/somadoc-engine/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/zhanghuanhuan303/somadoc-engine/releases/tag/v0.1.0
