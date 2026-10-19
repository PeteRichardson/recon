# About references and advanced filters

Most filters are one simple pattern. Two features help when a filter must do
more: a *reference* lets a pattern use a different filter's pattern, and an
*advanced filter* uses a more powerful engine.

## References: one pattern in one place

When two filters must agree about a pattern, a copy is a risk. Somebody
changes one copy and forgets the other. A reference prevents this.

In a pattern, `${set:name}` stands for the current pattern of the filter
`name` in the set `set`. `${name}` stands for a filter in the same set:

```toml
[[sets.team.filters]]
name = "Update summaries"
pattern = '${org:Summary}.*Updates'
```

recon expands the reference each time a pattern changes, so it always means
the current pattern of the other filter. A reference uses only the pattern.
It does not use the sense, the color or the on/off state of the other
filter.

When recon cannot find the filter that a reference names, the reference is
*broken*. In the window, the filter matches nothing and its row shows the
reason. In batch mode, a broken reference in a filter that is on stops the
run before any output.

## Advanced filters: a second engine

recon has two regular-expression engines. By default, a filter uses Rust's
`regex` engine. It is fast and it tests all filters in one pass, but it
cannot look ahead or behind.

Thus some questions are difficult to write. "A line that contains `Summary`
and `Updates`, in either order" needs lookahead. You can mark a filter as
*advanced*, with `engine = "advanced"` in the set file or `Ctrl-e` in the
[filter editor](filter-editor.md).

An advanced filter uses
[Oniguruma](https://github.com/kkos/oniguruma/blob/master/doc/RE), with its
Perl syntax. Oniguruma supports lookahead and lookbehind. It also has other
features that the standard engine does not have, for example backreferences,
atomic groups and conditionals. A lookbehind must have a fixed length:
`(?<=id=)` is correct, but `(?<=id=\d+)` is an error. The Oniguruma reference
describes its default syntax, so some details are different in recon.

This pattern uses two lookaheads:

```toml
pattern = '^(?=.*Summary)(?=.*Updates)'
engine = "advanced"
```

recon never makes a filter advanced without your decision, and its row in
the filter pane shows `[adv]`.

## The step limit

Oniguruma can backtrack, so some patterns take a very long time on some
lines. To prevent this, an advanced filter has a step limit. A line that
needs more steps than the limit counts as no match, and the filter's row
shows the number of lines that did this. The limit is a count of steps, not
a time, so a line gives the same result on each computer.

For the decision to use references and a second engine in place of a
language of clauses, see
[ADR 0008](https://github.com/PeteRichardson/recon/blob/main/docs/adr/0008-references-and-advanced-filters-not-clauses.md).
