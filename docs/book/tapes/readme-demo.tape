# The README's demo: browse, open a log, build a filter stack, then use it
# to move through the matches and the files.
Output docs/images/demo.gif
Set Width 1600
Set Height 820
Source docs/book/tapes/lib/common.tape

#> recon · a directory shows its contents in the view
Type "recon"
Sleep 300ms
Enter
Sleep 2s
#> j · JSON, shown through jq
Type "j"
Sleep 1.8s
#> j · Rust source, with syntax colour
Type "j"
Sleep 1.8s
#> k k · back to logs/
Type@500ms "kk"
Sleep 1s

#> l · go into logs/, then open a log
Type "l"
Sleep 1s
Type@400ms "jjj"
Sleep 600ms
Enter
Sleep 1.8s

#> ? · every key on one screen
Type "?"
Sleep 2.5s
Escape
Sleep 500ms

# Three ways to add a filter, three colours. The other lines are dimmed.
#> f i · include WARN
Type "fi"
Sleep 300ms
Type "WARN"
Sleep 300ms
Enter
Sleep 1.8s
#> :include ERROR · the same from the command palette
Type ":"
Sleep 300ms
Type "include ERROR"
Sleep 300ms
Enter
Sleep 1.8s
#> :context host= · colour a line, but do not keep it
Type ":"
Sleep 300ms
Type "context host="
Sleep 300ms
Enter
Sleep 1.8s

#> 01 · toggle filter 1 off, and on again
Type "01"
Sleep 1.5s
Type "01"
Sleep 1.5s
#> ! · every filter off, and back on
Type@1.5s "!!"
Sleep 1.5s

#> / then p · a search becomes a numbered filter
Type "/"
Sleep 300ms
Type "POST"
Sleep 300ms
Enter
Sleep 1.2s
Type "p"
Sleep 1.8s

#> u · hide the unmatched lines, and show them again
Type "u"
Sleep 2s
Type "u"
Sleep 1.5s

#> n / N · next and previous match
Type@700ms "nnn"
Sleep 500ms
Type@700ms "NN"
Sleep 1s

#> u then . and , · only the matches, file after file
Type "u"
Sleep 1.2s
Type@1.5s ".."
Sleep 1.5s
Type@1.5s ",,"
Sleep 2s
