# Panes

Panes are hidable and zoomable in-app or from the command line.
- Toggle zoom on the focused pane with `z`


## **Explorer**
- Navigate with arrows and h|j|k|l vim keys to select a file for viewing.
- Incremental search with `/`
- Highlighted lines indicate files that match an active include filter
- Reveal and Focus the explorer pane with `e`. Hide it with `E`

## **File View**
- display a view of the currently selected file.
- Cycle through configured views of a file type using `-`.
- Views are configurable, and include built-in (raw, hex, image) and user supplied (via pre-processing commands in config.toml.  e.g. running json through a jq command before displaying.)
- Reveal and Focus the file view with `t`.


## **Filter Pane**
- List Filtersets and Filters.
- Enable and disable filters individually or in groups by selecting one of the Filterset's 'profiles'.
- Reveal and Focus the filter pane with `f`. Hide it with `F`


{{#include glossary-links.md}}
