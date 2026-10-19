# A filter stack in one log: remove the noise, find the errors, hide the
# rest, and peek at the plain file.
Output docs/book/src/images/filter-stack.gif
Set Width 1400
Set Height 640
Source docs/book/tapes/common.tape

Type "recon logs/api-2026-10-15.log"
Sleep 300ms
Enter
Sleep 1.5s

# Exclude the health checks and the debug lines.
Type "fx"
Sleep 300ms
Type "/healthz"
Sleep 300ms
Enter
Sleep 1s
Type "fx"
Sleep 300ms
Type "DEBUG"
Sleep 300ms
Enter
Sleep 1s

# Include the errors: the other lines are dimmed.
Type "fi"
Sleep 300ms
Type "ERROR"
Sleep 300ms
Enter
Sleep 1.5s

# Hide the dimmed lines, then peek at the plain file and come back.
Type "u"
Sleep 2s
Space
Sleep 2s
Space
Sleep 2s
Type "q"
Sleep 500ms
