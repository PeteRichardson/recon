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

A set can also turn itself on at startup (`autoload`), and it can have a
priority that sets its position in the filter pane.

## Sets for a team

`RECON_FILTER_PATH` (or `--filter-path`) names more directories to read sets
from, separated by colons as in `PATH`. In each directory, recon reads
`filters.toml` and each `*.filters.toml` file. A team can keep its sets in a
shared repository, so that the knowledge about a log format is in one place.

When you have many sets, you do not need all of them on the screen. The *set
picker* (`L`) lists every set that recon found. In it, you select which sets
the filter pane lists. A set that is not listed has no effect on the view.

## Sets on the command line

`--set NAME` turns on a set at startup, and `--set NAME:PROFILE` turns on one
of its profiles. `--unlist NAME` removes a set from the filter pane. The same
options select the filters in [batch mode](batch_mode.md), so a set that you
made in the window also works in a script.
