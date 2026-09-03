# Configuration mechanism — design

**Status:** implemented. The schema was empty at first; the first settings
(`editor.project`, `editor.file`) landed with #42 — see
`docs/specs/2026-08-22-opening-an-editor.md`.
**Date:** 2026-08-22
**Issue:** #18

## Motivation

recon has no configuration mechanism. No file, no environment variables, no
persisted state.

The cost of that is not the missing settings — it is that the *first* setting to
need one will invent it, and the second will invent a different one. Two things
are already bent around the absence: #15 wants the `>>` selection marker to be
optional and has nowhere to put the option, and #8 needs persistence for saved
filter sets and will otherwise grow a private mechanism.

This spec decides the mechanism once, so that adding a setting is a small,
obvious edit rather than a design conversation.

**It deliberately added no settings.** The schema shipped empty, and every
candidate listed at the bottom lands in its own issue against this one. #42
was the first to do so, adding the `[editor]` table — including the first
*nested* section, and so the first exercise of `apply`'s exhaustive
destructure actually having something to merge. #62 added the second table,
`[filters]`, and with it the first setting that is a *list* and the first to
need step 4's range check as a real `deserialize_with` rather than a note.

Adding `[filters]` also surfaced a latent bug in `apply`: the `[editor]` merge
was written as `let ... else { return }`, which a second section would have
silently skipped whenever the file set `[filters]` but no `[editor]`. Each
section now merges inside its own `if let`. **Step 3 below is not enough on its
own** — the exhaustive destructure proves a field is *mentioned*, not that
control flow reaches it.

## Precedence

Highest wins:

```text
CLI flags  >  environment variables  >  config file  >  compiled-in defaults
```

Four layers, each one recon's own. A setting is resolved by taking the
highest layer that has an opinion about it.

`clap`'s `env` feature — now enabled in `Cargo.toml`, previously only `derive` —
resolves the first two boundaries with no code of ours:

```rust
#[arg(long, env = "RECON_NAV_WIDTH")]
pub nav_width: Option<u16>,
```

That leaves exactly one merge to write by hand, file-under-CLI, which lives in
`Config::apply`.

`figment` and `config` both implement the whole chain, and both presume a schema
large enough to pay for the dependency. recon's is empty. Revisit if the setting
count ever makes `apply` unpleasant; until then two small crates beat one large
one.

### `path` is CLI-only

The positional argument naming what to open is not a configurable setting and
never will be. It is a per-invocation fact, and a default for it in a file would
make bare `recon` open something other than the current directory — a surprise
no setting is worth.

### A setting may extend the chain below the file, but the chain stays four layers

#42's editor template reads `$VISUAL`/`$EDITOR` when recon has no template of
its own. Those are not `RECON_`-prefixed and are not recon's variables, so they
rank *below* the config file: a user with a global `EDITOR=vim` who has also
written a template into `config.toml` plainly meant the file to win.

```text
--editor  >  RECON_EDITOR  >  config file  >  $VISUAL / $EDITOR  >  compiled default
```

**Decision: this is a per-setting extension, not a fifth global layer.** The
four layers are uniform because each is recon's own and every setting has all
four. A foreign variable is a fact about one setting's *domain* — editors have a
Unix convention, nav widths do not — and modelling it as a global layer would
imply every setting has one, which is false.

The rule: a setting may define additional fallbacks beneath the file layer,
documented on that setting. It may never define one *above* the file layer,
because that is the boundary the user's own explicit configuration sits on.

## Format: TOML

Ratified in #18. The reasoning is factual rather than taste:

- **Not YAML.** `serde_yaml` is published as `0.9.34+deprecated`, and the
  community fork `serde_yml` is itself marked unmaintained. There is no
  maintained serde YAML backend to depend on.
- **Not JSON.** Parses fine, but has no comments — a poor fit for a file a human
  hand-edits and wants to annotate.
- **TOML** is the Rust ecosystem's default for exactly this, supports comments,
  and is one small dependency.

## The file is hand-edited only. recon never writes it

Ratified in #18, and enforced rather than merely documented:

```toml
toml = { version = "1.1.4", default-features = false, features = ["parse", "serde", "std"] }
```

Dropping default features drops `display`, which is the serializer. **`toml::to_string`
does not exist in this build.** The decision cannot be broken by accident,
because breaking it does not compile.

Consequences:

- No `toml_edit`. That crate exists to preserve comments, key order and
  whitespace across a write, and there is no write path.
- Config structs derive `Deserialize` only, never `Serialize`.
- #42's `--print-editor-config` stays a `println!`. It emits a stanza to paste
  and touches no file.

### If a write path ever lands

A settings-editor panel is plausible once there are enough settings to justify
one. The day it lands, this decision reverses — and doing it naively with
`toml::to_string` would **silently destroy every comment and all key ordering in
the user's file**, on a format chosen precisely because it supports comments.

So: any future write path must use `toml_edit`, never `toml`. Re-enabling the
`display` feature is the wrong fix and is the thing to catch in review.

## Location: `~/.config`, on every platform

```text
$XDG_CONFIG_HOME/recon/config.toml     when set to an absolute path
~/.config/recon/config.toml            otherwise
```

Resolved in `config_path_from`, which takes `$XDG_CONFIG_HOME` and `$HOME` as
arguments rather than reading them — see "Testing precedence" below.

**macOS gets `~/.config` too, deliberately.** The `directories` crate would
return `~/Library/Application Support/recon`, which is technically correct for a
macOS *application* and is not where a terminal tool's users look for a file
they hand-edit. Many Rust CLIs make the same call. recon is developed on macOS,
so this was decided on purpose rather than inherited from whichever crate got
added first; `macos_uses_dot_config_not_application_support` pins it as a test
so it cannot drift.

No crate is used at all. `etcetera` would let the strategy be named explicitly,
which is its own kind of documentation, but the strategy here is two environment
variable lookups and a `join`. The test above documents it more precisely than a
dependency would.

Details that follow the XDG base directory specification:

- A **relative** `$XDG_CONFIG_HOME` is invalid per the spec and is ignored.
  Honouring that matters: resolving it would make the config recon loads depend
  on the directory the shell happened to launch it from.
- An **empty** `$XDG_CONFIG_HOME` reads as unset. `export XDG_CONFIG_HOME=$SOMETHING_UNSET`
  is a common shell accident.
- **No `$HOME` and no `$XDG_CONFIG_HOME`** means there is no config path at all.
  That is not an error; recon runs on compiled-in defaults.

## Environment variables

`RECON_`-prefixed, one per setting, declared on the `#[arg]` that owns it. There
is no dynamic lookup and no way to set a value that has no flag — the flag *is*
the schema.

## When the file is wrong, recon refuses to start

**Decision: hard failure, not a warning.**

This is repo-specific and is the decision most likely to look over-strict out of
context. The argument is not "strictness is good", it is that the alternative
does not exist here:

> recon enters raw mode and the alternate screen immediately after config load.
> A warning printed and then continued past is **wiped off the screen a frame
> later**. "Warn and fall back" is "fall back silently" in practice.

So the error is printed on stderr, on the normal screen, before
`init_terminal()` — where the existing `[DEBUG] Config { .. }` line already
goes — and the process exits non-zero. The ordering in `main.rs` is load-bearing
and is commented as such.

Both error variants name the offending path. An error that says "invalid config"
without saying *which file* is close to useless when `$XDG_CONFIG_HOME` is in
play and the user is unsure which of two files recon actually found.

### Unknown keys are rejected

`#[serde(deny_unknown_fields)]`. A typo'd key in otherwise valid TOML —
`nav_wdith = 20` — would otherwise parse cleanly and the setting would simply
never apply. That is the most confusing configuration failure there is: you edit
the file, nothing changes, and there is no clue why. Loud is kinder.

What the user sees:

```text
invalid config file /Users/pete/.config/recon/config.toml
TOML parse error at line 1, column 1
  |
1 | nav_wdith = 20
  | ^^^^^^^^^
unknown field `nav_wdith`, there are no fields
```

The cost is that a config file written for a newer recon breaks an older one.
That is accepted: recon is a single-user tool with no version skew to speak of,
and the failure is loud and immediately fixable.

**"there are no fields" was correct while the schema was empty**, and the
message now names the real ones. `deny_unknown_fields` is applied to the
nested `[editor]` table too: a typo *inside* a section is exactly as invisible
as one at the top level, and exactly as worth reporting.

### What is *not* an error

- **A missing file.** Overwhelmingly the common case. Only `io::ErrorKind::NotFound`
  is forgiven; a permission error, or a directory sitting where the file should
  be, is reported as `ConfigError::Read`.
- **An empty file, or one containing only comments.** Comments are the reason
  TOML was chosen; a file that is nothing but comments must be valid.
- **No home directory to look in.**
- **`--help` and `--version` with a broken config file.** `clap` short-circuits
  these inside `Config::parse()`, before the file is read. Deliberate: help
  should work when everything else is broken.

### Values with a range, and settings that contradict each other

Not yet exercised — the schema is empty — but decided now so the first setting
with a range does not have to relitigate it:

- **An out-of-range value is a startup error**, same as a syntax error. `#33`'s
  nav-width floor is the motivating case: below `MIN_PANE_WIDTH` it is
  incoherent, above `MAX_NAV_WIDTH` it silently does nothing. Silently clamping
  is the same invisible failure as ignoring a typo'd key.
- **Cross-setting validation runs once, after the merge, in one place.** A floor
  configured above a configured ceiling is a contradiction the user wrote and
  should be told about, not one for the layout code to resolve at render time.
  Put it in `Config::validate`, called from `Config::load`, when the first such
  pair exists.

## Testing precedence

Environment variables are process-global, `std::env::set_var` is `unsafe` in
edition 2024, and this repo's tests run in parallel. It already carries a
`FIXTURE_NAMES` mutex in `fileview.rs` and a `FIXTURE_DIR_NAMES` mutex in
`lib.rs` for exactly this class of shared-state collision.

**The rule: don't set environment variables in tests.** Take the environment as
a parameter instead. `config_path_from(xdg, home)` is a pure function and its
seven tests run in parallel with no coordination at all; `config_path()` is the
thin wrapper that reads the real environment and is not itself tested.

Structure new settings the same way — the resolution logic in a function that
takes its inputs, the environment read at the edge.

When a test genuinely must set a real `RECON_*` variable (an end-to-end
precedence test through `clap` is the plausible case), serialize it on a shared
mutex the way `claim_fixture_name` does, and hold the guard across both the set
and the assertion.

Config file fixtures follow the existing house pattern: written under
`target/test-config/`, with `CONFIG_FIXTURE_NAMES` panicking loudly if two tests
claim the same name. Tests never touch the developer's real
`~/.config/recon/config.toml`.

## Adding a setting

1. Add the field to `Config` with `#[arg(long, env = "RECON_…")]`. This gets CLI
   and environment precedence, plus `--help`, for free.
2. Add the matching field to `FileConfig`, as an `Option<T>`.
3. `Config::apply` **stops compiling** — it destructures `FileConfig`
   exhaustively (`let FileConfig {} = file;`) precisely so that a field cannot be
   added to the file format and then silently never applied. Fix it by merging
   the new field: the file value applies only where the CLI/env layer did not
   already decide.
4. Add a range check if the value has one.
5. Test resolution with a pure function; test the file layer with a fixture.

## Settings to design against

Not implemented here. Collected so the mechanism is not designed in a vacuum,
and so each has somewhere to land:

| Candidate | Source | Note |
|---|---|---|
| ~~Filter palette~~ | #62 | **Landed.** `[filters] palette`, a whole-list replacement — the first setting that is neither a string nor a scalar, and the first to need a `deserialize_with` for a range check (`palette = []` divides by zero downstream) |
| ~~Syntax theme~~ | #122 | **Landed.** `[syntax] theme` and `--theme`/`RECON_THEME` — the first setting with a flag *and* a file key that resolve to a non-string value. Both run `syntax::Theme`'s `FromStr`, so a typo fails at parse time from either layer, and `none` is a value rather than an absence so the CLI can turn off what the file turned on |
| `>>` selection marker, optional | #15 | Blocked on this issue; the marker was removed outright in the meantime |
| Saved filter sets | #8 | TOML too, so there is one parser rather than two |
| Directory / executable colours | #15 | Style overrides |
| `HIDE_BADGE_TEXT` | #36 / PR #47 | Six columns is a real cost on a narrow row |
| `HIDE_BADGE_STYLE` | #36 / PR #47 | Bright yellow is deliberately loud and will fight some terminal themes |
| `PREVIEW_LINES`, `MAX_PREVIEW_BYTES` | `fileview.rs` | |
| `MIN_AUTO_NAV_WIDTH` | #33 | Numeric with a meaningful invalid range; interacts with `MAX_NAV_WIDTH` |
| `MAX_NAV_WIDTH` | `lib.rs` | The ceiling half of the pair above |
| Default hide mode at startup | | The toggle now survives file loads |
| Editor command template | #42 | The one setting whose fallback chain extends past the file |

## Out of scope

Every setting in that table. This spec delivers the mechanism, the precedence
rules, the file location, the error behaviour, and the record of why each was
chosen.
