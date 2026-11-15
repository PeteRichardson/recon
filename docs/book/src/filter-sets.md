# About filter sets

The filters that find a bug are usually three or four regular expressions.
To type them again in each session is slow, and it is easy to make an error.
A *filter set* keeps them in a file.

## Sets and profiles

A filter set is a named group of filters in a TOML file. recon reads
`~/.config/recon/filters.toml` at startup. In the filter pane, `S` saves the
filters that you typed as a new set.

A set can have *profiles*. A profile is a named list of the filters in the
set. When you select a profile, recon turns on only those filters. Thus one
set can serve different questions. For example, a Wi-Fi set can have one
profile for a roaming bug and a different profile for a disconnect bug.

`a` on a set's header opens the profile picker. Below the profiles, it also
lists the set's named filters. Select a filter to turn on only that filter,
so you do not need a profile for one filter.

A set can also turn itself on at startup (`autoload`), and it can have a
priority that sets its position in the filter pane.

## Sets for a team

`RECON_FILTER_PATH` (or `--filter-path`) names more directories to read sets
from, separated by colons as in `PATH`. In each directory, recon reads
`filters.toml` and each `*.filters.toml` file. A team can keep its sets in a
shared repository, so that the knowledge about a log format is in one place.

A `.include` or `.ignore` file in a directory selects which of its set files
recon reads: one glob for each line, matched against the file name. With a
`.include` file, recon reads only the files that it names. A file that a
`.ignore` line matches is not read, even when `.include` names it too. Thus a
large shared repository can sit on the path, and you read only the two files
that you use. A `[filter_dirs."<dir>"]` table in `config.toml` adds your own
`include` and `ignore` patterns to a directory's lists. The lists apply in
`~/.config/recon` too, but recon always reads `~/.config/recon/filters.toml`.

A set file can also have the `[commands]` and `[[filetype]]` rows of a
[command view](views.md#command-views). Such a file is a *pack*: one file
that adds a tool's filters, commands and views together.

When you have many sets, you do not need all of them on the screen. The *set
picker* (`L`) lists every set that recon found. In it, you select which sets
the filter pane lists. A set that is not listed has no effect on the view.

## Sets on the command line

`--set NAME` turns on a set at startup, and `--set NAME:PROFILE` turns on one
of its profiles. After the colon, a comma-separated list of profiles and
filter names applies from left to right, and `-` turns an item off:
`--set NAME:default,-noise` is the `default` profile without `noise`, and
`--set NAME:@all,-noise` is every filter but `noise`. `--set +NAME:ITEMS`
changes the state that an earlier `--set` gave the set, and does not replace
it. `--unlist NAME` removes a set from the filter pane. `--clear` removes
every set before `--set` runs, so `--clear --set NAME` lists only `NAME`;
`RECON_CLEAR=true` does the same. While recon runs, `:unlist @all` removes
every set the same way; see [the command palette](palette.md). The same
options select the filters in [batch mode](batch_mode.md), so a set that you
made in the window also works in a script.
