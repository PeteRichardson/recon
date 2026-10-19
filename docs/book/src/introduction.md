# Introduction

**recon** is a command line TUI tool for studying log files using combinations of regular expressions as filters. recon never modifies a file it views.

## Basic Features
- **Three pane UI**
  - Explorer: navigate the file system to pick a file to view
  - File View: study the file using combinations of regex filters
  - Filters: activate/deactivate filters and combinations of filters
- **Single key commands**
  - type `?` to see keymap
  - Keymap is configurable.
- **Mouse works** for selection, enabling filters, resizing panes, etc.
- **Include/Exclude/Context Filters**
  - Include filters specify a regex to match lines that are included in the results.
  - Exclude filters specify a regex to match lines that are excluded from the results.
  - Context Filters are always shown, but by themselves don't cause the filename to be highlighted in the explorer.
- **`<space>` peeks at unfiltered file**
- **`u` toggles unmatched lines from dim to hidden**
  - Unmatched lines are lines that match no filters (Include, Exclude or Context)
- **Incremental search** over the visible lines using `/`, `n` and `N`
  - Current search can be promoted to a filter with a keystroke ('p')
- **open selected file** in the appropriate editor/viewer using `o`
- **open selected file's project** in the configured editor using `O`


## Advanced Features
- [**Saved Filters**][filter] - store sets of filters in .filter.toml files that are found on $RECON_FILTER_PATH.
- **File highlighting in the Explorer pane** - files in the Explorer pane are colored if the file matches one or more of the active filters.  Useful to see which files in the current directory contain the patterns without opening each one.
- **Batch mode** - a command line can emit the lines (or files) that match a given set of filtersets and filterset profiles.
- **Command view** - Configure a custom shell command to preprocess files matching a MIME type and/or glob.  Filters apply to the command output.
- **Visual Filter Editor** - Create or Edit a filter with live highlighting on the currently viewed file.  Manually mark lines that must or must not match, and those lines will be checked against the pattern as you type it.  Marked lines are saved with the filter for re-validating filter edits.
- **Thumbnails for non-text files** (requires terminal support) - image files display in the file viewer.  On macOS, quicklook thumbnails are generated and displayed.
- **Simple and advanced Regex engines** - Default engine is Rust's regex crate.  Advanced (with backtracking) is [Oniguruma](https://github.com/kkos/oniguruma/blob/master/doc/RE).
- **Composable Filters** - Filters in filtersets can be named allowing the pattern to be re-used in other filters.  Useful if you want to combine or extend existing filters.
- **Syntax Highlighting** - uses syntect crate for parsing and theming.



{{#include glossary-links.md}}
