# ADR 0010: `-` is the lines on stdin

**Status:** accepted
**Date:** 2026-10-24

## Context

#353 made `-` as `PATH` read a **list of files** from stdin in batch mode,
one path per line. #439 made no `PATH` do the same when stdin was not a
terminal. So `find . -name '*.log' | recon -b` filtered the files `find`
named.

#517 asks for the other thing a pipe into recon can mean: the **lines** a
command writes, such as `sudo sfltool dumpbtm | recon --set btm -`. Both
meanings cannot have `-`. Every other Unix tool — `cat`, `grep`, `diff`,
`less` — reads `-` as the input's contents, not as a list of inputs.

## Decision

- **`-` as `PATH` is the lines on stdin**, in batch mode and (from the
  second half of #517) in the TUI. In batch mode they are filtered as they
  come, so an output that never ends works.
- **The file list moves to `--files-from FILE`**, with `-` as `FILE` for
  stdin, the name `rsync` and `tar` use.
- **Stdin is read only when asked.** With no `PATH` and no `--files-from`,
  the input is the current directory, pipe or not. This reverses the
  implicit read of #439.
- **`--emit files -` is refused** with a pointer to `--files-from -`, because
  that is what a script that used the old meaning most likely passed. A
  terminal on stdin with `-` is refused too: the read would wait for Ctrl-D.

This breaks scripts that piped a list into `-b` or into `-b -`. recon is
before 1.0, and the break is loud for `--emit files`.

## Considered options

- **`-` means lines in the TUI and a list in batch mode.** Rejected: the same
  argument would mean two things, and #517 wants lines in both.
- **Keep `-` for the list and add `--stdin` for lines.** Rejected: it keeps
  the meaning no other tool uses and gives the usual one a new name.
- **Keep the implicit list read with no `PATH`.** Rejected: whether stdin is
  a pipe is a guess about intent, and with two things a pipe can now mean,
  the guess is wrong half the time.
