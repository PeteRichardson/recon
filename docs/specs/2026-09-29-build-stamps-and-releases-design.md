# Build stamps and releases — design

**Date:** 2026-09-29
**Status:** approved by Pete Richardson on 2026-09-29, after a grilling session.
**Decision record:** [ADR 0004](../adr/0004-local-releases-and-build-stamps.md)
**Terms:** *release version*, *build stamp* and *release*, as `CONTEXT.md` defines them.

## 1. Why this document

recon has 441 commits and is still at 0.1.0. It has no tag and no release. Two problems follow from this:

1. **During work on a feature, it is not easy to know which code the running `recon` contains.** `~/bin/recon` is a symlink to `target/debug/recon` in the main checkout. When you test a PR in a worktree, `recon` on the PATH is not the worktree build. Today, the only sure fix is `cargo clean && cargo build`, which takes time.
2. **At a milestone, there is no process to mark it.** The 1.0 design (`2026-09-11-recon-1.0-release-design.md`, exit criterion 6) requires a tag and a GitHub release, but no document says how to make them.

This document solves both problems with the smallest amount of process that is possible.

## 2. The build stamp

### 2.1 What it shows

| Where | Content |
|---|---|
| `recon -V` | `recon 0.2.3+5.g1a2b3c4.dirty (PR68-Fix-I60-F3-rust-ps-path)` |
| `recon --version` | The `-V` line, then `checkout:`, `built:` and the existing `foundation-models:` line |
| Help/keymap screen | The same facts as `--version` |

Example of `recon --version`:

```
recon 0.2.3+5.g1a2b3c4.dirty (PR68-Fix-I60-F3-rust-ps-path)
checkout: /Users/pete/projects/recon/.claude/worktrees/PR68-…
built: 2026-09-29T11:32:07-07:00
foundation-models: on
```

The parts of the short line:

- `0.2.3` is the release version from `Cargo.toml`.
- `+5.g1a2b3c4` is SemVer build metadata in the `git describe` format: 5 commits after the last tag `vX.Y.Z`, and the abbreviated commit hash. If no tag is found, the part is `+g1a2b3c4`.
- `.dirty` shows that the tree has changes that are not committed.
- `(branch)` is the branch name. When `HEAD` is detached, it is `(detached)`.

A clean build that is exactly on a tag shows only `recon 0.2.3 (main)`, without a `+` part. (The branch stays, because it costs nothing and is always true.)

The build time is local time in RFC 3339 format, to the second.

### 2.2 When the stamp changes

A build script makes the stamp. The script runs again only when one of these changes:

- a file in the package (the Cargo default, which catches an edit that makes the tree dirty),
- `HEAD`, the ref that `HEAD` points to, or the git index (which catch a commit, a checkout, a reset and a `git add`).

Thus a `cargo build` with no changes does nothing and stays instant. The build time means "the time of the last real rebuild", which is the fact that use case 1 needs. The script must find the git directory with `git rev-parse --git-dir` and `--git-common-dir`, because in a worktree `.git` is a file and the refs are in the common directory.

### 2.3 With no git

When the build has no git repository (for example, a source tarball from a GitHub release) or no `git` program, the short line is `recon 0.2.3`, and `--version` shows `built:` and `foundation-models:` only. The build does not fail and does not print a warning.

### 2.4 Not in scope

recon does not warn at startup about a stale build or about a build from a different checkout. A quick look at `recon -V` finds these errors. See ADR 0004.

## 3. Releases

### 3.1 Source of truth

`Cargo.toml` holds the release version. A release commit `chore(release): vX.Y.Z` changes it (and `Cargo.lock` and `CHANGELOG.md`), and an annotated tag `vX.Y.Z` marks that commit. Only the `recon` package is released. The vendored `tui-textarea-2` workspace member keeps its own version.

### 3.2 The release skill

A general skill, `~/.claude/skills/release/`, runs the release from the main checkout. It is not specific to recon. Each repo keeps its own configuration: `release.toml` for `cargo-release` and `cliff.toml` for `git-cliff`.

Steps:

1. **Check.** Stop, and say why, unless all four are true. The skill does not repair anything.
   - The current directory is the main checkout (not a worktree), on `main`.
   - The tree is clean.
   - `main` is the same commit as `origin/main`, after a fetch.
   - The latest CI run for `HEAD` passed (`gh run list --commit <HEAD>`).
2. **Propose a level.** `git cliff --bumped-version` reads the Conventional Commits after the last tag. The skill shows the proposed version, the reason (for example "3 feat, 7 fix") and the notes, and the user confirms or gives another level (major, minor or patch).
3. **Release.** `cargo release <level> -p recon --execute`. A pre-release hook runs `git-cliff` to write the new `CHANGELOG.md` entry, so the entry is part of the release commit. `cargo-release` commits, tags and pushes `main` and the tag.
4. **Publish.** `gh release create vX.Y.Z --title vX.Y.Z --notes "<the new entry>"`. The skill gives the URL of the release.

If a push fails on SSH port 22, the skill tries again through `ssh.github.com:443` for that command only.

### 3.3 Level rules

| Commits after the last tag | Proposed level at 1.0 and after | Proposed level before 1.0 |
|---|---|---|
| A breaking change (`!` or `BREAKING CHANGE`) | major | minor |
| A `feat`, with no breaking change | minor | minor |
| Only other types | patch | patch |

Before 1.0, the skill never proposes major, because 1.0 has its own exit criteria. The user can still give major.

### 3.4 Release notes

`git-cliff` makes the notes from the Conventional Commits. The notes include only `feat` (as "Features"), `fix` ("Fixes"), `perf` ("Performance") and breaking changes ("Breaking changes", first). The notes do not include `docs`, `test`, `refactor`, `style`, `chore`, `build`, `ci` or commits that do not use the convention. A `#123` in a subject becomes a link to the issue. The same text goes into `CHANGELOG.md` and into the GitHub release body.

### 3.5 Not in scope

- No publish to crates.io (`publish = false`).
- No binaries attached to a release. Add `cargo-dist` when a person other than the author installs recon.
- No pre-releases (`-rc.N`) before 1.0.
- No release PR (release-please). See ADR 0004.

## 4. Baseline

`v0.1.0` is an annotated tag on the tip of `main` at the time the work is done. `CHANGELOG.md` starts with one entry:

```
## v0.1.0 — <date>

Baseline: the first tagged state. The changes before this tag are in the git history.
```

The baseline is done by hand, not by the skill, because there is no level to bump. After the baseline, the first run of the skill makes `v0.2.0` or `v0.1.1`.

## 5. Work items

| # | Item | Depends on |
|---|---|---|
| 1 | Build stamp: build script, `-V`, `--version` and the help/keymap screen (section 2) | — |
| 2 | Release: `release.toml`, `cliff.toml` and the general skill (section 3) | — |
| 3 | Baseline: tag `v0.1.0` and the first `CHANGELOG.md` (section 4) | 2, so that the changelog format agrees with `cliff.toml` |

## 6. Acceptance

1. In a worktree, `target/debug/recon -V` and `~/bin/recon -V` show different branches.
2. An edit to a source file followed by `cargo build` adds `.dirty` and a new `built:` time. A second `cargo build` with no changes does not compile recon again.
3. A commit followed by `cargo build` removes `.dirty` and shows the new hash.
4. A build from a copy of the tree with no `.git` succeeds and shows `recon X.Y.Z`.
5. The release skill stops with a clear message for each of the four failed checks.
6. After a release, `Cargo.toml`, the tag, `CHANGELOG.md` and the GitHub release all show the same version, and the notes contain no `docs`, `test` or `refactor` commits.
7. `v0.1.0` exists on GitHub, and `recon -V` on a clean `main` after the tag shows `0.1.0+N.g<hash>`.
