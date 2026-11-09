# A directory that cannot be read (tree/e, mode 000): the standard has chown() applied to each file met while
# walking, so ours changes the directory and then reports that it cannot be read; macOS does the same, GNU
# reports it without changing it. These lines differ in tree/e alone (checked by listing), -L and -P too.
-RP "$(othergroup)" tree
-Rh "$(othergroup)" tree
-R "$(othergroup)" .
-RL "$(othergroup)" tree
-RL "$(othergroup)" treelink
-RPL "$(othergroup)" treelink
-RPHL "$(othergroup)" treelink
# -H: a symbolic link met inside the hierarchy (not named as an operand). The standard's -H speaks of a link
# on the command line only; ours treats the others as -P does and changes the link itself, GNU (not macOS)
# changes what it leads to, outside the tree too.
-RH "$(othergroup)" tree
-RH "$(othergroup)" treelink
-RPH "$(othergroup)" treelink
# -h with -R and -H or -L: the synopsis has -h only without -R, so the combination is unspecified. -h wins here (the
# links themselves, no descent), GNU lets -H or -L win and walks the directory the link leads to; macOS refuses it.
-hRL "$(othergroup)" tree
-RhL "$(othergroup)" treelink
-RhH "$(othergroup)" treelink
