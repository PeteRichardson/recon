# About the filter editor

A regular expression is easy to write and difficult to make correct. A
pattern that is too wide includes the wrong lines. A pattern that is too
narrow does not include some of the correct lines. Usually you find these
errors only after you use the pattern. The filter editor helps you find them
while you write.

## Live results

`f I` opens the filter editor on a new filter, and `f C` opens it on the
selected filter. The editor covers the window. It shows the full file, and
highlights each line that the pattern matches after each key that you type.
A counter shows the number of matching lines in the full file.

When a pattern does not compile, the editor shows the error under the
pattern. The highlight stays as the last valid pattern made it, so a
half-typed `(` does not clear the screen.

## Checks and examples

You can mark a line as one that the pattern must match (`+`) or must not
match (`-`). A marked line is a *check*. recon tests each check after each
key, and shows which checks pass and which fail.

When you save the filter with `Enter`, recon keeps the checks with the filter
as *examples*. The next time that you edit the filter, the examples come back
as checks, also when the open file does not contain those lines. A change
that fails an example does not replace the pattern. Thus an example protects
a filter from a future edit that breaks it.

## A pattern from a description

On Apple silicon, recon has an optional build that uses Apple's on-device
model. In the filter editor, you type a request in plain language, for
example "the timeout errors". The model writes a pattern. recon shows the
pattern only when it compiles and passes each check. If it fails, recon sends
the failure back to the model and asks again, up to three times.

The model is a help, not an authority. Your checks decide if the pattern is
correct.

## Other features

The editor also keeps a history of versions of the pattern, so that you can
undo and redo. It can show only the matching and marked lines (`u`), and it
can make a filter [advanced](advanced-filters.md) (`Ctrl-e`).
