# Security Policy

## Supported versions

Security fixes are provided for the latest `0.1.x` release. Older versions are not maintained, so please
reproduce a report against the most recent release where possible.

| Version | Supported |
|---------|-----------|
| 0.1.x   | ✅        |
| < 0.1.0 | ❌        |

## Reporting a vulnerability

Please report security issues **privately** — do not open a public issue, pull request, or discussion for
a suspected vulnerability.

The preferred channel is a private vulnerability report through GitHub Security Advisories for this
repository: open the **Security** tab at `https://github.com/zhanghuanhuan303/somadoc-engine` and select
**Report a vulnerability**. This keeps the report visible only to you and the maintainers.

Please include, where you can:

- A description of the issue and its impact.
- The `somadoc-engine` version, your Rust version, and your operating system.
- A minimal proof of concept: the Markdown input, any `RenderRequest` fields involved, and the resulting
  behavior or output.

## What to expect

- We aim to acknowledge a report within a few days.
- We will investigate, keep you updated on our assessment, and let you know whether we consider it a
  vulnerability and how we plan to address it.
- We will credit you in the fix's release notes if you would like, and we are happy to coordinate on a
  disclosure timeline.

## Security model

`somadoc-engine` renders documents by compiling Typst **in-process** using the `typst` crate. There is no
subprocess, no shell, and no external sandbox, so the security boundary is the Typst compiler `World`
implementation that the engine provides.

- **No arbitrary file access.** The `World` (see `crates/somadoc-engine/src/svg_out.rs`) exposes only the
  document's main source plus two virtual paths: `/somadoc-logo` for an injected logo image and
  `/somadoc-img/<id>` for inline images supplied with the request. Any other path resolves to
  `FileError::NotFound`, so a document cannot read files from the host.
- **No command execution.** Typst has no command execution or arbitrary file I/O in this configuration, so
  a document cannot run shell commands or spawn processes.
- **No ambient environment values.** The `World` does not expose the current date or time, and font
  discovery is restricted to a whitelist. When `SOMADOC_FONTS_DIR` is set, only those colon-separated
  directories are scanned; otherwise the platform's standard font directories are used.
- **Template slugs are validated.** A template slug must match `[A-Za-z0-9_-]`, be non-empty, and must not
  start or end with `-`, as enforced by `valid_slug` in `crates/somadoc-engine/src/common.rs`. This
  prevents path traversal through `template_id` before any template file is resolved.

### Rendering untrusted input

Rendering is CPU- and memory-bound work, and a pathological document can be expensive to lay out. The
engine bounds this with a per-render timeout (`TypstEngine::timeout_ms`, 60 seconds by default) and runs the
synchronous compile on a blocking task so it does not stall the async executor. If you accept documents from
untrusted sources, keep a timeout in place and consider isolating rendering in a worker process as an
additional layer of defense.
