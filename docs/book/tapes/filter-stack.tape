# A filter stack in one log: remove the noise, find the errors, hide the
# rest, and peek at the plain file.
Output docs/book/src/images/filter-stack.gif
Set Width 1400
Set Height 640
Source docs/book/tapes/lib/common.tape

#> recon api-2026-10-15.log · open a log
Type "recon logs/api-2026-10-15.log"
Sleep 300ms
Enter
Sleep 1.5s

# Exclude the health checks and the debug lines.
#> f x · add an exclude filter: the health checks
Type "fx"
Sleep 300ms
Type "/healthz"
Sleep 300ms
Enter
Sleep 1s
#> f x · exclude the DEBUG lines too
Type "fx"
Sleep 300ms
Type "DEBUG"
Sleep 300ms
Enter
Sleep 1s

# Include the errors: the other lines are dimmed.
#> f i · add an include filter: ERROR, and the rest is dimmed
Type "fi"
Sleep 300ms
Type "ERROR"
Sleep 300ms
Enter
Sleep 1.5s

# Hide the dimmed lines, then peek at the plain file and come back.
#> u · hide mode: only the matching lines
Type "u"
Sleep 2s
#> space · peek at the plain file
Space
Sleep 2s
#> space · back to the filtered view
Space
Sleep 2s
#> q · quit
Type "q"
Sleep 500ms
