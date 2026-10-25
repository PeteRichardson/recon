# About the command palette

Push `:` to open the [command palette](glossary.md) on the bottom row. Type a
command and its argument, then push Enter. Esc closes the palette and changes
nothing.

`:` works in the explorer, the file view and the filter pane. It does not
work in the filter editor, the set picker or a `/` prompt, because there you
can type `:` in a regular expression. Push Esc, then `:`.

## Go to a folder or a file

`:e /Volumes/MyDisk/files` sets the explorer's [folder](glossary.md). `:cd`
is the same command.

- A directory: the explorer lists it and selects the first entry after `..`,
  the same as when you step into it with the keys.
- A file: the explorer lists the folder of the file, selects the file and
  opens it in the file view, the same as a path on the command line.
- `~` is your home folder. A relative path, for example `../logs`, starts at
  the explorer's folder.
- `:e` alone goes to your home folder. `:e -` goes back to the last folder.
- The explorer shows and gets the focus.

recon's working directory does not change. Thus `O` and the paths that you
gave on the command line keep their meaning.

## Go to a line

`:goto 120` or `:120` puts the cursor on line 120 of the file. This is the
number that the line numbers show, not a count of the visible lines. If a
filter hides line 120, the cursor goes to the next visible line, and the
status line tells you.

## Enable or unlist a filter set

`:set DeployLogs` lists the [filter set](glossary.md) `DeployLogs` if it is
unlisted, and enables it with its `default` profile. This is the same as
`--set DeployLogs` on the command line, and you do not open the set picker.

`:set DeployLogs:triage,timeouts` enables the set with the profiles and the
named filters of the list, as `--set NAME:LIST` does. The other filters of
the set are off.

`:unlist DeployLogs` unlists the set, as `--unlist` does.

If recon does not know the set or an item of the list, the palette stays
open and shows the error.

## Run a keymap action

Each action of the keymap is a command, with the name that a `[keymap]` line
uses, for example `:global.follow` or `:explorer.down`. An action that you
removed the key from is still a command. A pane's action works on its pane,
and that pane shows and gets the focus. `:q` quits, as `q` does.

## Find a command

Above the line, a list shows the commands that agree with what you typed.
Each row shows the names, the argument, the key and a line of help. If the
line is empty, the list shows all the commands.

- The start of a name agrees first, then the letters of a name in order:
  `gfol` finds `global.follow`.
- The help text agrees too: `folder` finds `e, cd`.
- Tab and Shift-Tab choose the next or the previous row and write its name in
  the line. On a path, Tab completes the name of a file or a directory.
- After `set` or `unlist`, Tab completes the name of a known set. After
  `set NAME:` and after each `,`, Tab completes a profile or a named filter
  of that set.
- Up and Down recall the command lines that ran. The palette keeps its own
  history for the session.
