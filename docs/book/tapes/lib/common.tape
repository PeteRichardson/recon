# The look and the start of each tape. Each tape sets its own Width and
# Height first, because VHS ignores a Set after the Type lines below.
# make-gifs.sh copies the demo data to /tmp/recon-demo.
Set Shell bash
Set Framerate 15
Set FontSize 15
Set TypingSpeed 80ms

Hide
Type "cd /tmp/recon-demo && export PS1='$ ' && clear"
Enter
Show
