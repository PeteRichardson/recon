# The filter editor: type a pattern and see each match, then mark lines
# that the pattern must match. A failed check is red until the pattern
# is correct.
Output docs/book/src/images/filter-editor.gif
Set Width 1400
Set Height 640
Source docs/book/tapes/common.tape

Type "recon logs/api-2026-10-15.log"
Sleep 300ms
Enter
Sleep 1.5s

# From the explorer, so that the editor opens with no marked line.
Type "e"
Sleep 300ms
Type "fI"
Sleep 1s
Type "timed out"
Sleep 1.5s

# On the lines: mark the next match, then the 504 under it.
Tab
Sleep 500ms
Type "n"
Sleep 800ms
Type "+"
Sleep 800ms
Down
Sleep 500ms
Type "+"
Sleep 2s

# The 504 line fails. Correct the pattern, and the check passes.
Shift+Tab
Sleep 500ms
Type "|504"
Sleep 2s
Enter
Sleep 2s
Type "q"
Sleep 500ms
