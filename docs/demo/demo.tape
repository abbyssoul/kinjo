# Kinjo demo recording, played against the built-in sample backend, so no real
# device, address, or hardware id appears on screen.
#
# Record with `scripts/record-demo.sh`, which builds the binary this tape runs
# and isolates it from the recording machine's own configuration.

Output docs/demo/kinjo-demo.gif

Set Shell "bash"
Set FontSize 16
Set Width 1400
Set Height 760
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 60ms
Set WindowBar Colorful

Hide
Type "export PS1='$ ' && clear"
Enter
Show

Type "kinjo --backend fake"
Sleep 400ms
Enter
# Let the sample services stream in.
Sleep 3s

# Browse: the details pane follows the selection.
Up@500ms 3
Sleep 2s
Up@500ms 2
Sleep 2s

# Fuzzy search.
Type "/"
Sleep 300ms
Type@150ms "wopr"
Sleep 1s
Enter
Sleep 500ms
Down
Sleep 2.5s
Sleep 500ms
Type "/"
Ctrl+U
Escape
Sleep 1s

# Switch views: hosts, then service types.
Tab
Sleep 2s
Tab
Sleep 1.5s
Type "/"
Type@150ms "ssh"
Sleep 1s
Enter
Down@400ms 3
Sleep 1.5s

# An `ssh {hostname}` rule over two hosts asks which one to act on.
Enter
Sleep 3s
Escape
Sleep 500ms
Type "/"
Ctrl+U
Escape
Sleep 1s

# Commands view.
Tab
Sleep 2s

Type "?"
Sleep 3s
Escape
Sleep 500ms
Type "q"
Sleep 1s
