# ADR 0004: Releases are a local command, and a build stamp is SemVer build metadata

**Status:** accepted
**Date:** 2026-09-29

## Context

After 441 commits, recon was still at 0.1.0, with no tag and no release. Two
needs were open. During work on a feature, it was not easy to know if the
`recon` that runs has the latest code, especially in a worktree, where
`~/bin/recon` is the build of the main checkout. At a milestone, there was no
process to change the release version, tag it and publish notes.

## Decision

- **`Cargo.toml` is the source of truth for the release version.** A release
  commit changes it, and an annotated tag `vX.Y.Z` marks that commit. The
  build does not read the version from a tag.
- **A release is one local command.** A general skill runs `cargo release`
  from the main checkout. It stops unless the checkout is on `main`, the tree
  is clean, `main` is the same as `origin/main` and CI is green for `HEAD`.
  `git-cliff` makes the notes from the Conventional Commits (`feat`, `fix`,
  `perf` and breaking changes only), for `CHANGELOG.md` and for the GitHub
  release. The skill proposes major, minor or patch from the commits, and the
  user confirms it. The release does not publish to crates.io, and there are
  no pre-releases before 1.0. `v0.1.0` tags the state before this ADR as a
  baseline.
- **The build stamp is SemVer build metadata.** `recon --version` shows
  `X.Y.Z+N.gHASH[.dirty] (branch)`. The long form and the help screen also
  show the checkout path and the build time. A build that is exactly on a tag
  and clean shows `X.Y.Z` only. With no git, the stamp is the release version
  and the build time. The build time changes only when the build script runs
  again (a source file, `HEAD` or the index changed), so a build with no
  changes stays instant.

## Alternatives rejected

- **release-please.** It keeps a release PR open and does the release when
  the PR merges. This repo goes to `main` by a local fast-forward, and a
  release PR is one more thing to manage at each release.
- **A datetime as a fourth version part** (`0.2.3.26-09-29T11:32`). This is
  not valid SemVer, so cargo rejects it. Also, a datetime alone does not show
  if the code is current: a rebuild of old code gets a new time.
- **A build time that is always new.** It makes recon's crate compile again
  on each `cargo build`, when there are no changes too.
- **A warning at startup for a stale build or a build from a different
  checkout.** A quick look at `--version` finds most of these errors, and the
  warning adds complexity for a small gain.
