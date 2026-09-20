# Contributing to somadoc-engine

Thanks for your interest in `somadoc-engine` — a pure-Rust engine that compiles Markdown
(plus handwritten LaTeX) into Typst and renders it to PDF or per-page SVG. This guide covers
how to build, test, and extend the project, and what we expect from contributions.

## Prerequisites

- Rust, edition 2021. The minimum supported Rust version (MSRV) is **1.85**.
- No external toolchain. The engine compiles Typst in-process, so you do not need `pandoc`,
  a LaTeX distribution, or a system sandbox to build or run the tests.

## Repository layout

| Path | Purpose |
|------|---------|
| `crates/somadoc-core` | IR data model, IR-to-Markdown serialization, text extraction |
| `crates/somadoc-engine` | Markdown → Typst → PDF / SVG engine; `TypstEngine` implements `RenderEngine` |
| `crates/somadoc-engine/src/typst.rs` | Source composition, template loading, and the public `typeset` / `typeset_svg` paths |
| `crates/somadoc-engine/src/latex_commands.rs` | Handwritten LaTeX → Typst translation layer |
| `crates/somadoc-engine/src/common.rs` | Template slug validation and template resolution |
| `crates/somadoc-engine/templates/<slug>/template.typ` | Typst templates, for example `crates/somadoc-engine/templates/soma/template.typ` |

## Build, test, format, and lint

```sh
cargo build
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets
```

- **Build:** `cargo build` compiles the workspace. A release build (`cargo build --workspace --release`)
  is what CI uses, so it is worth checking before you open a pull request.
- **Test:** `cargo test --workspace` runs the unit tests in both crates. New behavior should come with
  tests; bug fixes should come with a test that fails before the fix.
- **Format:** run `cargo fmt --all` before committing. We follow rustfmt defaults, and pull requests must
  keep `cargo fmt --all --check` passing.
- **Lint:** keep `cargo clippy --workspace --all-targets` free of warnings. Avoid silencing a lint with
  `#[allow(...)]` unless there is a clear, documented reason.

## Continuous integration

Every push to `main` and every pull request runs the GitHub Actions workflow in
`.github/workflows/ci.yml` on `ubuntu-latest`:

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo build --workspace --release --locked`
- `cargo test --workspace --locked`
- `cargo doc --workspace --no-deps --locked`

The job installs `fonts-noto-cjk` first, so the CJK rendering tests are deterministic.

**CI must pass before a pull request can be merged.** If CI fails, treat it as a real signal: bisect and
fix the underlying cause rather than disabling or deleting the failing test. Running the same commands
locally first will usually save a round trip.

## Adding a template

Templates are resolved relative to the directory passed to `TypstEngine::new(...)`. In this repository
that directory is the crate's `templates/`, so a template with the slug `rubric` lives at
`crates/somadoc-engine/templates/rubric/template.typ`.

To add a template:

1. Create `crates/somadoc-engine/templates/<slug>/template.typ`. The `<slug>` must be non-empty, contain only
   `[A-Za-z0-9_-]`, and must not start or end with `-`. This is enforced by `valid_slug` in
   `crates/somadoc-engine/src/common.rs` to prevent path traversal, so an invalid slug is rejected
   before any file is read.
2. Define the entry point:

   ```typst
   #let article(title: none, authors: (), lang: "en", toc: false, body) = {
     set page(paper: "a4", margin: 1in)
     body
   }
   ```

   The engine composes the translation of the document body with this function, so keep the signature
   stable.
3. Read customization passed from the Markdown YAML frontmatter through the injected `_somadoc_vars`
   dictionary, for example `_somadoc_vars.at("mainfont", default: "")`. Keys containing `-` are
   normalized to `_`, since `-` is not valid in a Typst identifier.
4. Use `crates/somadoc-engine/templates/soma/template.typ` as a complete, working reference — it exercises headings, lists,
   quotes, booktabs tables, footnotes, CJK fonts, a running header, and an optional `/somadoc-logo` image.
5. Add or extend a test for the new template so its behavior is covered.

A template directory must contain a `template.typ` entry point; the engine rejects a slug whose
`template.typ` does not exist.

## Adding a LaTeX command mapping

The translation layer lives in `crates/somadoc-engine/src/latex_commands.rs`. As its module documentation
states, the matcher functions `named_to_typst` and `parse_control_symbol` are the authoritative command
list: they are organized by category (text styles, colors, sizes, spacing and breaks, math fonts, and
so on).

To add a command:

1. Add the mapping in the matching category of `named_to_typst` (for `\name`-style commands) or
   `parse_control_symbol` (for control symbols). If the command takes arguments, register its arity in
   `command_arity` as well so the scanner knows how many braced groups to consume.
2. Add a unit test in the `mod tests` module at the bottom of the same file, asserting the exact Typst
   output produced. Follow the style of the existing cases such as `text_style_commands` and
   `color_commands`.

**Both the matcher and its unit test must be updated together.** A new mapping without a test will be
asked for changes during review, because the translation output is what document rendering depends on.

## Upgrading Typst

The engine embeds `typst` as a library, so a Typst upgrade is an API migration rather than a version
bump. Two constraints make that sharper than usual:

1. **The family moves together.** `typst`, `typst-svg`, `typst-pdf`, and `typst-assets` must stay on the
   same minor version. Cargo will happily resolve `typst 0.13` next to `typst-svg 0.15`, but the graph
   then contains two `typst-library` versions and the build fails on type mismatches between them
   (`E0308`, `E0593`, `E0599`) — for example `typst_svg::svg` expects the 0.15 `typst_layout::Page`
   while the engine passes the 0.13 `typst::layout::Page`.
2. **`mdxport` pins Typst too.** `mdxport` depends on `typst = "0.13"`, so it keeps pulling the 0.13
   family even when this crate moves on. That is tolerable only because no Typst types cross the
   `mdxport` boundary — the engine uses `mdxport` for frontmatter parsing and Markdown → Typst
   conversion, not for compilation. Note the one exception: rendering *without* a `template_id` falls
   back to `mdxport::markdown_to_pdf`, which compiles with `mdxport`'s own Typst version.

To upgrade:

1. Bump `typst`, `typst-svg`, `typst-pdf`, and `typst-assets` together in the root `Cargo.toml`.
2. Run `cargo build --workspace` and fix the API changes. Expect to start with the `World`
   implementation in `crates/somadoc-engine/src/svg_out.rs`, which implements `typst::World` directly,
   and then the `typst_svg` / `typst_pdf` call sites.
3. Run the full suite (`cargo test --workspace`). The tests in `crates/somadoc-engine/src/typst.rs`
   render real PDFs and SVGs, so they catch both compile- and layout-level regressions.
4. Re-render a document with the reference template and read the output over.

Because these bumps are breaking, Dependabot is configured to ignore `typst*` minor updates (see
`.github/dependabot.yml`); patch releases are still proposed automatically.
`typst::utils::LazyHash` is a re-export of the internal `typst-utils` helper crate, so it is usable but
sits outside Typst's documented stability surface — re-check it on every upgrade.

Prefer upstreaming a change you need (or working around it at the Typst-source level, as the engine
already does for booktabs bottom rules and thematic-break width) over forking Typst. The project uses
upstream `typst` from crates.io with no `[patch]` and no git dependency, and a fork would add a
substantial maintenance burden.

## Commit messages

We use [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>
```

Common types are `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, and `chore`. Keep the
description in the imperative mood (for example, `feat(latex): add \boxed mapping`). The scope is optional
but helpful — `typst`, `svg`, `latex`, `templates`, and `core` are good candidates.

## Pull requests

- Keep each pull request **focused**: one logical change, with unrelated refactors or reformatting left out.
- Link the issue the change addresses in the description.
- Confirm that `cargo test --workspace` and `cargo fmt --all --check` pass, and that
  `cargo clippy --workspace --all-targets` is clean.
- Update documentation and `CHANGELOG.md` when the change is user-visible.

## License

By contributing, you agree that your contributions are licensed under the Apache-2.0 license, the same
terms that cover this project. See `LICENSE` for the full text.
