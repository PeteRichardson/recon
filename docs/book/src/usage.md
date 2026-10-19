```
Usage: recon [OPTIONS] [PATH]

Arguments:
  [PATH]  File or directory to open. A directory is listed with its first entry selected; a file is opened with its own directory listed alongside. With
          none, the current directory

Options:
      --editor <TEMPLATE>                Command template `O` runs, e.g. `zed {project} {file}:{line}` [env: RECON_EDITOR]
      --file-editor <TEMPLATE>           Command template `o` runs for a text file that no `[[filetype]]` row hands off. Defaults to `--editor` with the
                                         `{project}` argument dropped, so one setting normally configures both keys [env: RECON_FILE_EDITOR]
      --clipboard <COMMAND>              Command `y` pipes a selection to, e.g. `pbcopy` or `xclip -selection clipboard`. Falls back to `[clipboard]
                                         command` in `config.toml`, then to the platform's own tool [env: RECON_CLIPBOARD]
      --print-editor-config [<FLAVOUR>]  Print a ready-to-paste `[editor]` stanza and exit. Takes a flavour — `zed`, `vscode`, `wezterm-nvim`, … — or
                                         `auto` to guess from `$TERM_PROGRAM` [possible values: auto, zed, vscode, sublime, idea, terminal-nvim,
                                         iterm-nvim, wezterm-nvim, kitty-nvim, ghostty-nvim]
      --print-keymap [<WHICH>]           Print the keymap as a ready-to-paste `[keymap]` stanza and exit [possible values: effective, defaults]
      --background <BACKGROUND>          Whether the terminal background is dark or light: picks the built-in filter palette and the grey of dimmed
                                         lines. Falls back to a top-level `background` in `config.toml`, then to `dark` [env: RECON_BACKGROUND]
                                         [possible values: dark, light]
      --filter-path <DIRS>               More directories to read filter sets from, colon-separated, e.g. `~/team-filters:./.recon`. recon reads
                                         `filters.toml` and each `*.filters.toml` in them [env: RECON_FILTER_PATH]
      --theme <THEME>                    Colours for the file view's syntax colouring: a bundled theme name, a path to a `.tmTheme` file, or `none` to
                                         turn colouring off [env: RECON_THEME]
  -e, --emit <WHAT>                      Print the session's result to stdout on `q`; `Q` quits without it [possible values: lines, files, cwd]
  -b, --batch                            Batch mode: no TUI. Print the result of `--emit` (`lines` if not given) for the files on stdin or under PATH,
                                         then exit
  -n, --line-numbers                     With `--emit lines`: prefix each line with its line number and a tab
  -s, --set <NAME[:LIST]>                Enable a saved filter set at startup, as `NAME` for its `default` profile or `NAME:LIST` for the filters a
                                         comma-separated list of profiles and filter names turns on. Repeatable
  -u, --unlist <NAME>                    Unlist a saved filter set at startup: no row in the filter pane and no effect on the view or on `--emit`.
                                         Repeatable
      --hide                             Start in hide mode: only matching lines and files
      --hide-pane <PANES>                Panes to hide at startup, comma-separated: `explorer`, `view`, `filters`. Replaces `[layout] hide_panes` in
                                         `config.toml`. At least one pane stays shown [possible values: explorer, view, filters]
  -q, --quiet                            Suppress the summary line on stderr; warnings still print
  -w, --warnings                         Show startup warnings (keymap, views). On unless something turns them off
      --no-warnings                      Hide the startup warnings for this run. The opposite of `--warnings`
  -h, --help                             Print help (see more with '--help')
  -V, --version                          Print version
```
