# Build stamps and releases — design

**Date:** 2026-09-29
**Status:** approved by Pete Richardson on 2026-09-29, after a grilling session. Updated on 2026-10-13 with the findings of the pre-flight in [PeteRichardson/vrsn](https://github.com/PeteRichardson/vrsn) (see the comments on #337, #338 and #339).
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
| `recon -V` | `recon 0.2.3+5.g1a2b3c4.dirty (Fix-I60-F3-rust-ps-path)` |
| `recon --version` | The `-V` line, then `checkout:`, `built:` and the existing `foundation-models:` line |
| Help/keymap screen | The same facts as `--version` |

Example of `recon --version`:

```
recon 0.2.3+5.g1a2b3c4.dirty (Fix-I60-F3-rust-ps-path)
checkout: /Users/pete/projects/recon/.claude/worktrees/Fix-I60-…
built: 2026-09-29T11:32:07-07:00
foundation-models: on
```

The parts of the short line:

- `0.2.3` is the release version from `Cargo.toml`.
- `+5.g1a2b3c4` is SemVer build metadata in the `git describe` format: 5 commits after the last tag `vX.Y.Z`, and the abbreviated commit hash. If no tag is found, the part is `+g1a2b3c4`.
- `.dirty` shows that the tree has changes that are not committed.
- `(branch)` is the branch name. When `HEAD` is detached, it is `(detached)`.

A release build — clean, and exactly on a `vX.Y.Z` tag — shows only `recon 0.2.3`, with no `+` part and no branch. Such a build contains exactly the release code, so the branch says nothing about the code. The `checkout:` and `built:` lines in `--version` still say which checkout made the build. Thus the test for a real release is simple: `recon -V` has no `+` and no `(…)`.

All other cases keep the branch: a dirty tree on a tag, commits after a tag, and no tag.

A CI build shows `+g<hash> (detached)`, because `actions/checkout` makes a shallow clone with no tags and a detached `HEAD`. This is correct. A test must not expect a tag or a branch name, or it fails in CI.

The build time is local time in RFC 3339 format, to the second.

### 2.2 When the stamp changes

A build script makes the stamp. The script runs again only when one of these changes:

- `src/`, `build.rs`, `Cargo.toml` or `Cargo.lock` (which catch an edit that makes the tree dirty),
- `HEAD`, the ref that `HEAD` points to, or the git index (which catch a commit, a checkout, a reset and a `git add`),
- `<git-common-dir>/packed-refs` and `<git-common-dir>/refs/tags` (which catch a new tag, and a ref that `git gc` moved into `packed-refs`).

The script must name all of these paths with `cargo:rerun-if-changed`. When a build script prints one `rerun-if-changed` line, Cargo stops its default check of the package files, so the script must name `src/` and the other package files itself.

The script names a path only when it exists. A `rerun-if-changed` path that does not exist makes the script run again on every build. After `git gc`, the loose ref file of `HEAD` can be gone, with the ref only in `packed-refs`.

The script runs `git --no-optional-locks status`, not a plain `git status`. A plain `git status` can write the index when it refreshes stale stat data (after a `touch` or a checkout). Then the index is newer than the build, and the next build compiles recon again for no reason. The option also stops the script from taking `index.lock` while you run a git command of your own.

Thus a `cargo build` with no changes does nothing and stays instant. The build time means "the time of the last real rebuild", which is the fact that use case 1 needs. The script must find the git directory with `git rev-parse --git-dir` and `--git-common-dir`, because in a worktree `.git` is a file and the refs are in the common directory.

A manual `git status` in a shell can still write the index when the stat data is stale, and the build script cannot prevent it. Then the next build compiles recon again one time. A real edit does not cause this, and the dependencies are never compiled again.

### 2.3 With no git

When the build has no git repository (for example, a source tarball from a GitHub release) or no `git` program, the short line is `recon 0.2.3`, and `--version` shows `built:` and `foundation-models:` only. The build does not fail and does not print a warning.

### 2.4 Not in scope

recon does not warn at startup about a stale build or about a build from a different checkout. A quick look at `recon -V` finds these errors. See ADR 0004.

## 3. Releases

### 3.1 Source of truth

`Cargo.toml` holds the release version. A release commit `chore(release): vX.Y.Z` changes it (and `Cargo.lock` and `CHANGELOG.md`), and an annotated tag `vX.Y.Z` marks that commit. Only the `recon` package is released. The vendored `tui-textarea-2` workspace member keeps its own version. Check that `cargo release -p recon` does not change it. If it does, add `[package.metadata.release]` with `release = false` to `vendor/tui-textarea-2/Cargo.toml`. (vrsn has one package, so the pre-flight did not test this.)

recon's `Cargo.toml` needs `publish = false` in `[package]`. Without it, `cargo-release` requires `description`, `license` and `repository` or `homepage`, and stops with an error.

Each repo keeps two files:

- `release.toml`: a copy of [vrsn's `release.toml`](https://github.com/PeteRichardson/vrsn/blob/main/release.toml), which has no repo-specific values. It releases only from `main`, with tag `v{{version}}`, commit `chore(release): v{{version}}`, `publish = false`, and a pre-release hook that writes `CHANGELOG.md` with `git-cliff`.
- `cliff.toml`: a copy of [vrsn's `cliff.toml`](https://github.com/PeteRichardson/vrsn/blob/main/cliff.toml), with the issue link URL changed from vrsn to recon. Keep its whitespace settings: `header = "# Changelog\n"`, `trim = false`, and a body that ends with `{% endfor %}\`. With other settings, `--prepend` gives zero or two blank lines between entries.

recon's CI needs no change. It runs on `push: branches: [main]` and on pull requests, so a tag push does not start a run.

### 3.2 The release skill

A general skill, `~/.claude/skills/release/` (`SKILL.md` and `preflight.sh`, in PeteRichardson/skills), runs the release from the main checkout. It already exists, and it is not specific to recon. Each repo keeps its own configuration: `release.toml` for `cargo-release` and `cliff.toml` for `git-cliff`.

Steps:

1. **Check.** Stop, and say why, unless all four are true. The skill does not repair anything.
   - The current directory is the main checkout (not a worktree), on `main`.
   - The tree is clean.
   - `main` is the same commit as `origin/main`, after a fetch.
   - The latest CI run for `HEAD` passed (`gh run list --commit <HEAD>`).
2. **Propose a level.** `git cliff --bumped-version` reads the Conventional Commits after the last tag. The skill shows the proposed version, the reason (for example "3 feat, 7 fix") and the notes, and the user confirms or gives another level (major, minor or patch).
3. **Release.** `cargo release <level> -p recon --execute --no-confirm --no-push`. A pre-release hook runs `git-cliff` to write the new `CHANGELOG.md` entry, so the entry is part of the release commit. `cargo-release` commits and tags, but does not push.
4. **Push.** `git push --atomic origin main vX.Y.Z`. The skill does the push, not `cargo-release`, so that a push that fails can be tried again with no second version bump. If the push fails on SSH port 22, the skill tries again through `ssh.github.com:443` for that command only.
5. **Publish.** `gh release create vX.Y.Z --title vX.Y.Z --notes "<the new entry>"`. The skill gives the URL of the release.

### 3.3 Level rules

| Commits after the last tag | Proposed level at 1.0 and after | Proposed level before 1.0 |
|---|---|---|
| A breaking change (`!` or `BREAKING CHANGE`) | major | minor |
| A `feat`, with no breaking change | minor | minor |
| Only other types | patch | patch |

Before 1.0, the skill never proposes major, because 1.0 has its own exit criteria. The user can still give major.

"Only other types → patch" needs code in the skill. When the notes leave out all commits after the tag, `git cliff --bumped-version` returns the current version, not a patch. The skill proposes patch in that case. `git cliff --context` lists only the commits that go in the notes, so the skill counts the left-out commits as all commits after the tag minus the listed ones.

### 3.4 Release notes

`git-cliff` makes the notes from the Conventional Commits. The notes include only `feat` (as "Features"), `fix` ("Fixes"), `perf` ("Performance") and breaking changes ("Breaking changes", first). The notes do not include `docs`, `test`, `refactor`, `style`, `chore`, `build`, `ci` or commits that do not use the convention. A `#123` in a subject becomes a link to the issue. The same text goes into `CHANGELOG.md` and into the GitHub release body.

### 3.5 Not in scope

- No publish to crates.io (`publish = false`).
- No binaries attached to a release. Add `cargo-dist` when a person other than the author installs recon.
- No pre-releases (`-rc.N`) before 1.0.
- No release PR (release-please). See ADR 0004.

## 4. Baseline

`v0.1.0` is an annotated tag on the tip of `main` at the time the work is done. `CHANGELOG.md` starts with `# Changelog`, one blank line, then one entry, so that it agrees with the `cliff.toml` header and later entries are prepended with one blank line:

```
# Changelog

## v0.1.0 — <date>

Baseline: the first tagged state. The changes before this tag are in the git history.
```

The baseline is done by hand, not by the skill, because there is no level to bump. After the baseline, the first run of the skill makes `v0.2.0` or `v0.1.1`.

Do the baseline in two steps, so that the tagged state includes the changelog:

1. A PR adds `CHANGELOG.md`, in a commit of type `docs`, so that the release notes leave it out. Use the date of the tag. If the PR merges on a different day, change the date before the merge. The PR body says "Part of #339". It must not say "Close", "Fixes" or "Resolves" with the issue number anywhere, also not as a next step, because GitHub then closes the issue at the merge, before the tag exists.
2. After the PR merges and CI passes on the merge commit, tag the merge commit and make the GitHub release by hand, then close #339:

   ```bash
   git tag -a v0.1.0 -m v0.1.0 <merge commit>
   git push origin v0.1.0
   printf 'Baseline: the first tagged state. The changes before this tag are in the git history.\n' \
     | gh release create v0.1.0 --title v0.1.0 --verify-tag --notes-file -
   ```

The commits before the tag are part of the baseline. A `feat` that must go in the first real release notes must merge after the tag.

## 5. Work items

| # | Item | Depends on |
|---|---|---|
| 1 | Build stamp: build script, `-V`, `--version` and the help/keymap screen (section 2) | — |
| 2 | Release: `release.toml`, `cliff.toml` and `publish = false` (section 3). The general skill already exists | — |
| 3 | Baseline: tag `v0.1.0` and the first `CHANGELOG.md` (section 4) | 2, so that the changelog format agrees with `cliff.toml`; 1, so that the new tag changes the stamp at once |

## 6. Acceptance

1. In a worktree, `target/debug/recon -V` and `~/bin/recon -V` show different branches.
2. An edit to a source file followed by `cargo build` adds `.dirty` and a new `built:` time. A second `cargo build` with no changes does not compile recon again, also after `git gc`.
3. A commit followed by `cargo build` removes `.dirty` and shows the new hash.
4. A build from a copy of the tree with no `.git` succeeds and shows `recon X.Y.Z`.
5. The release skill stops with a clear message for each of the four failed checks.
6. After a release, `Cargo.toml`, the tag, `CHANGELOG.md` and the GitHub release all show the same version, and the notes contain no `docs`, `test` or `refactor` commits. `cargo release -p recon` does not change `vendor/tui-textarea-2`.
7. `v0.1.0` exists on GitHub. On a clean `main` at the tag, the next `cargo build` shows `recon 0.1.0` with no new commit. After N more commits, `recon -V` shows `recon 0.1.0+N.g<hash> (main)`.
