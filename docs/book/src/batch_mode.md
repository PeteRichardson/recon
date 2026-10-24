# About output and batch mode

recon is a part of the shell, not a replacement for it. When you find
something in recon, the next step is often a different command: `sort`,
`xargs` or `cd`. recon can give its result to that command.

## Output when you quit

recon draws its window on stderr, so stdout is free for a result. With
`--emit`, recon prints a result when you quit with `q`:

| `--emit` | Prints |
| --- | --- |
| `lines` | the visible lines of the file view |
| `files` | the files in the explorer, one absolute path on each line |
| `cwd` | the directory that the explorer shows |

`Q` quits without output, and exits with status 1. Thus a shell script or a
`cd` does not continue after you stop a session.

The display mode decides the result. Dim mode prints each line or file;
hide mode prints only the matches. The output cannot show which mode was
on, so recon always prints a summary line to stderr that names the mode and
the counts. Batch mode always prints in hide mode.

## Batch mode

In batch mode (`-b`), recon does not open a window. It reads files from the
command line or from stdin, applies the filter sets that `--set` names, and
prints the result:

```sh
recon -b --set errors server.log
ls -1 *.log | recon -b --emit files --set BugFilters:Bug57
```

Batch mode always uses hide mode: it prints only the matching lines, or only
the matching files. In batch mode there is no grey and no `u` key, so dim
mode would hide the matches among the other lines. `--hide` is still
accepted, but in batch mode it does nothing.

Batch mode uses the same filter sets as the window. Thus you can make a set
by experiment in the window, and then use it without change in a script or
a CI job. `-n` puts the original line number before each line, separated by
a tab.

![Batch mode in the shell](images/batch.gif)

## Follow a growing file

With `--follow` (`-f`), batch mode prints the matching lines of one file,
and then each new matching line when it comes, as `tail -f | grep` does
with your filter sets. recon flushes each line at once, so the next command
in a pipe gets it immediately. Push `Ctrl-c`, or send SIGTERM, to stop:

```sh
recon -b -f --set errors server.log | tee errors.log
```

If the file becomes shorter or a different file replaces it (logrotate),
recon reads it again from the start and writes a message on stderr.
recon refuses `--follow` with a directory, with more than one file, and
with an `--emit` other than `lines`.

For the reason that batch mode has its own flag, see
[ADR 0005](https://github.com/PeteRichardson/recon/blob/main/docs/adr/0005-batch-mode-has-a-flag.md).
For the reason that batch mode always hides, see
[ADR 0009](https://github.com/PeteRichardson/recon/blob/main/docs/adr/0009-batch-mode-always-hides.md).
