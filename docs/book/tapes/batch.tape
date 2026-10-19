# Batch mode: the same filter set, with no window, in a pipe.
Output docs/book/src/images/batch.gif
Set Width 1100
Set Height 520
Source docs/book/tapes/common.tape

Type "recon -b --emit files --set triage --hide logs"
Sleep 300ms
Enter
Sleep 2s

Type "recon -b -n --set triage:slow --hide logs/api-2026-10-13.log"
Sleep 300ms
Enter
Sleep 2.5s

Type "recon -b -q --set triage:slow --hide logs | wc -l"
Sleep 300ms
Enter
Sleep 2.5s
