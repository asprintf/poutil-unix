# owner[:group] as the standard has it: a user name or a number, and after a colon a group name or a
# number. GNU takes more (":group", "user:", ":", "", a plus sign or a blank in front of a number); these
# are errors here.
: f
'' f
" $(id -u)" f
":" f
":$(id -g)" f
"$(id -un):" f
+$(id -u) f
# A directory that cannot be read (tree/e, mode 000): the standard has chown() applied to each file met
# while walking, so ours changes the directory and then reports that it cannot be read; GNU reports it
# without changing it.
-hR "$(id -u):$(id -G | cut -d' ' -f2)" tree
-R -H -L "$(id -u):$(id -G | cut -d' ' -f2)" tree
-R -P "$(id -u):$(id -G | cut -d' ' -f2)" tree
-R "$(id -u):$(id -G | cut -d' ' -f2)" .
-R "$(id -u):$(id -G | cut -d' ' -f2)" ./
-R "$(id -u):$(id -G | cut -d' ' -f2)" d/..
-R "$(id -u):$(id -G | cut -d' ' -f2)" nosuch tree
-R "$(id -u):$(id -G | cut -d' ' -f2)" tree
-R "$(id -u):$(id -G | cut -d' ' -f2)" tree nosuch
-R "$(id -u):$(id -G | cut -d' ' -f2)" tree tree
-R "$(id -u):$(id -G | cut -d' ' -f2)" tree tree/s
-R "$(id -u):$(id -G | cut -d' ' -f2)" tree/s tree
-R "$(id -u):$(id -G | cut -d' ' -f2)" treelink/
-Rh "$(id -u):$(id -G | cut -d' ' -f2)" tree
# -H: a symbolic link met inside the hierarchy (not named as an operand). The standard's -H says nothing
# about it; ours treats it as -P does and changes the link itself, GNU changes what it leads to.
-R -L -H "$(id -u):$(id -G | cut -d' ' -f2)" tree
-R -P -H "$(id -u):$(id -G | cut -d' ' -f2)" treelink
-RH "$(id -u):$(id -G | cut -d' ' -f2)" tree
-RH "$(id -u):$(id -G | cut -d' ' -f2)" treelink
-RH "$(id -u):$(id -G | cut -d' ' -f2)" treelink/
-RLH "$(id -u):$(id -G | cut -d' ' -f2)" treelink
-RPH "$(id -u):$(id -G | cut -d' ' -f2)" treelink
# -h with -H or -L: the options exclude each other and -h wins here (the link itself is meant); GNU
# refuses the combination.
-RhH "$(id -u):$(id -G | cut -d' ' -f2)" treelink
-RHh "$(id -u):$(id -G | cut -d' ' -f2)" treelink
-RhL "$(id -u):$(id -G | cut -d' ' -f2)" treelink
# The same unreadable directory (tree/e) with the other option letters.
-RHL "$(id -u):$(id -G | cut -d' ' -f2)" treelink
-RL "$(id -u):$(id -G | cut -d' ' -f2)" tree
-RL "$(id -u):$(id -G | cut -d' ' -f2)" treelink
-RP "$(id -u):$(id -G | cut -d' ' -f2)" tree
-RPL "$(id -u):$(id -G | cut -d' ' -f2)" treelink
