# Browsing: the filters stay when the file changes, so the explorer marks
# the logs that have errors, and hide mode removes the others.
Output docs/book/src/images/browsing.gif
Set Width 1400
Set Height 640
Source docs/book/tapes/common.tape

# The triage set: include ERROR, exclude the health checks and DEBUG.
Type "recon --set triage logs"
Sleep 300ms
Enter
Sleep 2s

# Each file shows only what the filters chose. A dim name has no errors.
Type@1.2s "jjjjj"
Sleep 1.2s

# Back to a log with errors, then hide mode: only the files with errors
# stay in the explorer.
Type "k"
Sleep 1.2s
Type "u"
Sleep 2s
Type "k"
Sleep 2s
Type "q"
Sleep 500ms
