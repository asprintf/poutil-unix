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
# -L: a trailing blank continues the line only when it is unescaped ("unless the last character of the line is an
# unescaped <blank>"); GNU continues after "a\ " as well
in:t_esc -L 1 sh -c 'echo "$#:$*"' x
# -s: "a command line length less than size", with the null byte of each string and of the utility counted, so a line of
# exactly size bytes does not fit; GNU (and the BSD man page) allow size bytes. Each line below was checked by running ours
# with the size raised by one against GNU with the size as written: stdout, status and stderr then agree.
in:s4 -s 10 echo
in:s4 -s 10 -x echo
in:s4 -s 10 -n 2 echo
in:s4 -s 15 echo
in:s4 -s 15 -x echo
in:s4 -s 15 -n 2 echo
in:s4 -s 15 -n 2 -x echo
in:s4 -s 20 echo
in:s4 -s 20 -x echo
in:s4 -s 25 echo
in:s4 -s 25 -x echo
in:s4 -s 25 -I {} echo '{}'
in:s6 -s 9 echo
in:s6 -s 9 -x echo
in:s6 -s 9 -n 2 echo
in:s6 -s 9 -n 2 -x echo
in:s6 -s 11 echo
in:s6 -s 11 -x echo
in:s6 -s 13 echo
in:s6 -s 13 -x echo
in:s6 -s 15 echo
in:s6 -s 15 -x echo
in:s3l -s 10 echo
in:s3l -s 10 -x echo
in:s3l -s 10 -n 2 echo
in:s3l -s 10 -I {} echo '{}'
in:s3l -s 15 echo
in:s3l -s 15 -x echo
in:s3l -s 15 -n 2 echo
in:s3l -s 15 -n 2 -x echo
in:s3l -s 20 echo
in:s3l -s 20 -x echo
in:s_abc -s 9 echo
in:s_abc -s 15 echo
in:s_abc -s 15 -x echo
in:s10 -s 16 echo
in:s10 -s 16 -x echo
in:s10 -s 16 -I {} echo '{}'
in:s_three4 -s 10 echo
in:s_three4 -s 10 -x echo
in:s_three4 -s 15 echo
in:s_three4 -s 15 -x echo
in:s_three4 -s 20 echo
in:s_three4 -s 20 -x echo
in:long_line -s 4096 sh -c 'echo "$#"' x
# -x with an argument that cannot fit even alone: ours runs the command line gathered before it and then stops with an
# error ("an argument that cannot fit alone is an error before anything after it runs"); GNU with -x stops before running
# the line gathered so far. The spec only says -x terminates "if a constructed command line will not fit".
in:s_abc -s 8 -x echo
in:s_abc -s 9 -x echo
in:s_abc -s 10 -x echo
in:s_abc -s 11 -x echo
in:s_abc -s 12 -x echo
in:s_abc -s 13 -x echo
in:s_abc -s 14 -x echo
in:s_long -s 8 -x echo
in:s_long -s 9 -x echo
in:s_long -s 10 -x echo
in:s_long -s 11 -x echo
in:s_long -s 12 -x echo
in:s_long -s 13 -x echo
in:s_long -s 14 -x echo
in:s_long -s 15 -x echo
in:s_long -s 16 -x echo
in:s_long -s 20 -x echo
in:s_long -s 25 -x echo
in:s_long -s 30 -x echo
# -n without -s: "Fewer arguments shall be used if the command line length accumulated exceeds the size specified by the -s
# option (or {LINE_MAX} if there is no -s option)", so -n 100 is cut at {LINE_MAX} (2048 here) and -x with it stops;
# GNU fills the line up to the system limit
in:line60 -n 100 sh -c 'echo "$#"' x
in:line60 -n 100 -x sh -c 'echo "$#"' x
in:long_line -n 1000 sh -c 'echo "$#"' x
# -n without -s: the size it implies is {LINE_MAX} (2048), and -x terminates "if a constructed command line will not fit in the
# implied or specified size", also when it is the first argument alone, or the utility's own, that is over it. Ours stops with
# an error before running anything; GNU takes only the real limit of the system. (Without -x the argument is run: there is
# nothing to cut.)
in:big3000 -n 1 -x sh -c 'echo "$#"' x
in:big3000 -n 2 -x sh -c 'echo "$#"' x
in:big3000 -n 3 -x sh -c 'echo "$#"' x
in:one_nl -n 1 -x echo "$(head -c 3000 /dev/zero | tr '\0' x)"
