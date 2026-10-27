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

`:set +DeployLogs:-timeouts` starts from the filters that the pane shows now
for the set, and turns `timeouts` off. Without the `+`, the list starts with
all filters of the set off. The items apply from left to right: `-` before
an item turns its filters off, and `@all` is every filter of the set.

`:unlist DeployLogs` unlists the set, as `--unlist` does.

If recon does not know the set or an item of the list, the palette stays
open and shows the error.

## Choose a view

`:view hex` shows the open file in the hex [view](views.md). A view of the
file type's cycle steps the cycle there, as `-` does, and it holds for every
file of that type. Any other view that recon knows, for example a command
view that no `[[filetype]]` row names, shows only the open file. When you
open another file, or push `-`, the cycle's view comes back.

If recon does not know the view, the palette stays open and shows the error.

## Add a filter

`:include ERROR` adds an including [filter](glossary.md) with the pattern
`ERROR` to the scratch set, as `i` and Enter do in the filter pane.
`:exclude RE` adds an excluding filter, and `:context RE` adds a context
filter. The pattern is the rest of the line, so it can have spaces in it.
You do not open the filter pane, and the focus does not move.

If the pattern is not a valid regular expression, the palette stays open and
shows the error. No filter is added.

## Run a keymap action

Each action of the keymap is a command, with the name that a `[keymap]` line
uses, for example `:global.follow` or `:explorer.down`. An action that you
removed the key from is still a command. A pane's action works on its pane,
and that pane shows and gets the focus. `:q` quits, as `q` does.

## Run your own aliases

An alias gives a name to a chain of command lines. Write it in the
`[aliases]` table of `config.toml`, with `;` between the lines:

```toml
[aliases]
proj = "e ~/projects/recon ; explorer.focus"
errs = "set +errors:-WARNING"
go = "explorer.set $1 ; explorer.focus"
```

`:proj` runs `e ~/projects/recon`, then `explorer.focus`, as if you typed
each line. All of a line after the command name is its argument, so the
argument can have spaces in it. It cannot have a `;` in it, because `;` ends
the line.

An alias can take arguments. `$1` to `$9` in the chain are the words that you
type after its name, so `:go ~/projects` runs `explorer.set ~/projects`, then
`explorer.focus`. The last argument gets all of the rest of the line, so a
value with a space in it stays one argument. `$*` is all of the line after
the `$<n>` arguments, and it can be empty. A `;` in a value that you type
stays in the value. Write `$$` for a `$`. Tab completes the last argument as
the command that takes it does: a path for `explorer.set $1`.

- Each alias has its own row in the list, after the built-in commands, with
  its chain as the help.
- With too few arguments, no line runs, and the palette names the argument
  that is missing. Text after an alias with no arguments is an error.
- If a line fails, the chain stops there. The lines before it stay done, and
  the palette shows the error with the alias and the line.
- recon does not start if an alias has a line that names no command, an empty
  line, the name of another alias, or `$2` but not `$1`. An alias cannot run
  another alias yet.
  recon also does not start if the name of an alias is the name of a command
  (for example `e` or `global.follow`), a number, or a name with a space or a
  `;` in it. The message names the alias.

To give an alias a key, write its name in `[keymap]`, as you do for an action:

```toml
[keymap]
proj = 'F6'
```

The key works in every pane and runs the chain, as `:proj` does. If the chain
fails, the palette opens with the error. A key cannot give arguments, so the
key of an alias that takes them opens the palette with the name of the alias
typed. Then you type the arguments. The `?` overlay shows the key in the
**Aliases** section. If the key is a default key of an action, the alias gets
it, and recon tells you. If another `[keymap]` line also binds the key, or the
line names no action and no alias, recon does not start.

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
- After `view`, Tab completes a view name: the views of the file's cycle
  first, then the other views.
- Up and Down recall the command lines that ran. The palette keeps its own
  history for the session.
