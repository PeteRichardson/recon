# Keys

The default keys that this guide names. `?` in recon shows the full keymap.
`recon --print-keymap` prints it in the format of the configuration file.

## Global keys

| Key | Action |
| --- | --- |
| `?` | Show the keymap |
| `e` / `t` / `f` | Show and focus the explorer / file view / filter pane |
| `E` / `T` / `F` | Hide the explorer / file view / filter pane |
| `z` | Zoom the focused pane, or show the other panes again |
| `space` | Peek: turn off all filters and change the display mode; press again to restore |
| `!` | Turn off all filters; press again to restore |
| `u` | Change between dim mode and hide mode |
| `/` | Search the visible lines, or the file names in the explorer |
| `n` / `N` | Next / previous hit; with no search, next / previous interesting line |
| `p` | Change the current search into an include filter |
| `-` | Show the next view of the file |
| `Ctrl-t` | Follow the file as it grows, or stop |
| `o` | Hand off the file to the app for its type |
| `O` | Open the project that contains the file in the editor |
| `q` | Quit, and print the `--emit` result |
| `Q` | Quit without the `--emit` result |

## Filter keys

| Key | Action |
| --- | --- |
| `f i` / `f x` | Add an include / exclude filter |
| `f c` | Edit the pattern of the selected filter |
| `f I` / `f C` | Open the filter editor on a new / the selected filter |
| `Enter` | In the filter pane: turn the selected filter on or off |
| `m` | In the filter pane: change the selected filter between include and context |
| `S` | In the filter pane: save the typed filters as a new set |
| `L` | Open the set picker |
