# Vendored fork of tui-textarea-2

**Upstream:** https://github.com/srothgan/tui-textarea
**Vendored version:** 0.12.1, copied verbatim from crates.io
**Licence:** MIT (see LICENSE, unmodified)

## Why this fork exists

`recon` dims lines that do not match a filter and colours those that do. The
public API exposes only whole-area, cursor-line, search and line-number
styles — there is no way to style an individual line. The machinery already
exists internally (`LineHighlighter::set_line_style`, used for the cursor
line); this fork only makes it reachable.

## Changes from upstream

None yet — this commit is a verbatim copy, so that the wiring can be verified
independently of any patch.

## Rebasing onto a new upstream release

1. Copy the new version over this directory.
2. Re-apply the entries listed above (each names its file and anchor).
3. Run `cargo test` in the repo root; all tests must pass.

The patch is deliberately confined to per-line *presentation*. If it ever
needs to touch cursor movement, `screen_map.rs` or `wrap.rs`, stop: that is
the signal to reconsider a purpose-built viewer instead (see the spec).
