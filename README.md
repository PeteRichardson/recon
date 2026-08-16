# list

A TUI log viewer with a two-pane file navigator.

## Vendored dependency

`vendor/tui-textarea-2` is a patched copy of
[tui-textarea-2](https://github.com/srothgan/tui-textarea) 0.12.1, wired in
via `[patch.crates-io]`. The patch adds two public setters — per-line styles
and gutter number overrides — which the file view needs in order to dim lines
that do not match a filter and to show original line numbers while filtered.

See `vendor/tui-textarea-2/PATCH.md` for the exact changes and how to rebase
onto a new upstream release, and `upstream.patch` for the diff as offered
upstream. Note that `upstream.patch` also contains one local-only hunk
(removal of `[profile.bench]` from `Cargo.toml`, needed only because this
crate lives inside `recon`'s workspace) that `PATCH.md` explicitly calls out
as not for upstream submission; drop that hunk before sending the patch
anywhere else. If the rest of the patch is accepted, this directory and the
`[patch.crates-io]` entry can both be deleted.
