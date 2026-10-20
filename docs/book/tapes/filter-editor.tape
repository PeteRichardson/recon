# The filter editor: type a pattern and see each match, then mark lines
# that the pattern must match. A failed check is red until the pattern
# is correct.
Output docs/book/src/images/filter-editor.gif
Set Width 1400
Set Height 640
Source docs/book/tapes/lib/common.tape

#> recon api-2026-10-15.log · open a log
Type "recon logs/api-2026-10-15.log"
Sleep 300ms
Enter
Sleep 1.5s

# From the explorer, so that the editor opens with no marked line.
#> e · focus the explorer
Type "e"
Sleep 300ms
#> f I · open the filter editor on a new filter
Type "fI"
Sleep 1s
#> type a pattern: each match is highlighted
Type "timed out"
Sleep 1.5s

# On the lines: mark the next match, then the 504 under it.
#> Tab · move to the lines
Tab
Sleep 500ms
#> n · next match
Type "n"
Sleep 800ms
#> + · the pattern must match this line
Type "+"
Sleep 800ms
#> + · and this one: the check fails (red)
Down
Sleep 500ms
Type "+"
Sleep 2s

# The 504 line fails. Correct the pattern, and the check passes.
#> Shift-Tab · back to the pattern
Shift+Tab
Sleep 500ms
#> add |504: the check passes
Type "|504"
Sleep 2s
#> Enter · add the filter
Enter
Sleep 2s
#> q · quit
Type "q"
Sleep 500ms
