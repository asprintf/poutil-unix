# -I, -L and -n exclude each other and the spec leaves open which one counts. Ours: the last of -L and -n,
# and -I stays whatever follows (as macOS does). GNU lets a later -L or -n replace -I... but only for -L
# here; it warns on stderr in all of these.
in:lines -L 2 -n 1 sh -c 'echo "$#:$*"' x
in:lines -n 1 -L 2 sh -c 'echo "$#:$*"' x
in:three -n 1 -I {} echo '<{}>'
in:three -I {} -L 1 echo '<{}>'
# -s larger than the system takes is not an error (spec); GNU warns about it on stderr
in:many -s 10000000000000 sh -c 'echo "$#"' x
# NUL cannot be in an argument and the input is a text file; ours refuses it, GNU warns and cuts there
in:nul sh -c 'echo "$#:$*"' x
