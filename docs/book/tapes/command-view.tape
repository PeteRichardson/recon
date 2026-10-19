# A command view: recon shows a JSON file through jq, and a filter
# operates on the lines that jq writes, not on the bytes on disk.
Output docs/book/src/images/command-view.gif
Set Width 1400
Set Height 640
Source docs/book/tapes/common.tape

Type "recon orders.json"
Sleep 300ms
Enter
Sleep 2s

# Include the failed order, then hide the other lines.
Type "fi"
Sleep 300ms
Type "failed|error"
Sleep 300ms
Enter
Sleep 1.5s
Type "u"
Sleep 2s

# The raw view: the file on disk is one line, so the filter matches it all.
Type "-"
Sleep 2.5s
Type "-"
Sleep 1.5s
Type "q"
Sleep 500ms
