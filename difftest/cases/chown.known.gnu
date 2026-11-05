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
# -h with -H or -L: the synopsis has -h only without -R, so the combination is unspecified. -h wins here (the link
# itself is meant, no walk); GNU 4.9 does not refuse it, -H or -L win there and the directory is walked.
-RhH "$(id -u):$(id -G | cut -d' ' -f2)" treelink
-RHh "$(id -u):$(id -G | cut -d' ' -f2)" treelink
-RhL "$(id -u):$(id -G | cut -d' ' -f2)" treelink
# The same unreadable directory (tree/e) with the other option letters.
-RHL "$(id -u):$(id -G | cut -d' ' -f2)" treelink
-RL "$(id -u):$(id -G | cut -d' ' -f2)" tree
-RL "$(id -u):$(id -G | cut -d' ' -f2)" treelink
-RP "$(id -u):$(id -G | cut -d' ' -f2)" tree
-RPL "$(id -u):$(id -G | cut -d' ' -f2)" treelink
# GNU takes "user.group" (a period for the colon, the old BSD form) when there is no colon; the spec
# knows only the colon, so a period makes "1.5" a user name that does not exist here
1.5 f
# "user:" and "user." are no owner[:group] here (a group portion that is empty is no group name, a period is a
# name character); GNU takes "user:" as the login group of the user, and a period as the colon (with a warning)
nobody: f
. f
# Run as an ordinary user (root can read tree/e): the unreadable directory tree/e (mode 000) in a hierarchy. The standard has
# chown() applied to each file met while walking, so ours changes the directory and then reports that it cannot be read;
# GNU reports it without changing it. Every line below was run for both and the listings compared: they differ in tree/e
# alone, except the lines with -H, which also differ as the -H block says, and the lines with -h and -L (the -h block).
# With -L a link that leads nowhere is reported by both, with other words (ours adds the cycle tree/s/up).
-R "$(id -u):$(id -G | cut -d' ' -f2)" tree sl dang f
-hR "$(id -u):$(id -G | cut -d' ' -f2)" tree sl dang f
-Rh "$(id -u):$(id -G | cut -d' ' -f2)" tree sl dang f
-RP "$(id -u):$(id -G | cut -d' ' -f2)" tree sl dang f
-hRH "$(id -u):$(id -G | cut -d' ' -f2)" tree
-hRH "$(id -u):$(id -G | cut -d' ' -f2)" tree sl dang f
-hRP "$(id -u):$(id -G | cut -d' ' -f2)" tree
-hRP "$(id -u):$(id -G | cut -d' ' -f2)" tree sl dang f
-RLP "$(id -u):$(id -G | cut -d' ' -f2)" tree
-RLP "$(id -u):$(id -G | cut -d' ' -f2)" tree sl dang f
-R -P "$(id -u):$(id -G | cut -d' ' -f2)" tree sl dang f
-R -h "$(id -u):$(id -G | cut -d' ' -f2)" tree
-R -h "$(id -u):$(id -G | cut -d' ' -f2)" tree sl dang f
-hRL "$(id -u):$(id -G | cut -d' ' -f2)" tree
-hRL "$(id -u):$(id -G | cut -d' ' -f2)" tree sl dang f
-h -R -L "$(id -u):$(id -G | cut -d' ' -f2)" tree
-h -R -L "$(id -u):$(id -G | cut -d' ' -f2)" tree sl dang f
-RHL "$(id -u):$(id -G | cut -d' ' -f2)" tree
-R -L "$(id -u):$(id -G | cut -d' ' -f2)" tree
-RL "$(id -u):$(id -G | cut -d' ' -f2)" tree sl dang f
-RHL "$(id -u):$(id -G | cut -d' ' -f2)" tree sl dang f
-R -L "$(id -u):$(id -G | cut -d' ' -f2)" tree sl dang f
-RLH "$(id -u):$(id -G | cut -d' ' -f2)" tree
-RPH "$(id -u):$(id -G | cut -d' ' -f2)" tree
-R -H "$(id -u):$(id -G | cut -d' ' -f2)" tree
-RH "$(id -u):$(id -G | cut -d' ' -f2)" tree sl dang f
-RLH "$(id -u):$(id -G | cut -d' ' -f2)" tree sl dang f
-RPH "$(id -u):$(id -G | cut -d' ' -f2)" tree sl dang f
-R -H "$(id -u):$(id -G | cut -d' ' -f2)" tree sl dang f
-R -H "$(id -u):$(id -G | cut -d' ' -f2)" treelink
-R -L "$(id -u):$(id -G | cut -d' ' -f2)" treelink
# -H and a symbolic link met inside the hierarchy, as above: ours changes the link itself (as -P does), GNU changes what
# it leads to, outside the tree too, and reports a link that leads nowhere (status 1)
-RH "$(id -u):$(id -G | cut -d' ' -f2)" t3
-RLH "$(id -u):$(id -G | cut -d' ' -f2)" t3
-RPH "$(id -u):$(id -G | cut -d' ' -f2)" t3
-R -H "$(id -u):$(id -G | cut -d' ' -f2)" t3
-RH "$(id -u):$(id -G | cut -d' ' -f2)" top3
-RLH "$(id -u):$(id -G | cut -d' ' -f2)" top3
-RPH "$(id -u):$(id -G | cut -d' ' -f2)" top3
-R -H "$(id -u):$(id -G | cut -d' ' -f2)" top3
-RH "$(id -u):$(id -G | cut -d' ' -f2)" toptop3
-RLH "$(id -u):$(id -G | cut -d' ' -f2)" toptop3
-RPH "$(id -u):$(id -G | cut -d' ' -f2)" toptop3
-R -H "$(id -u):$(id -G | cut -d' ' -f2)" toptop3
-RH "$(id -u)" top3
# -h with -R and -H or -L: the synopsis has -h only without -R, so this is unspecified. Ours lets -h win (the link
# itself, no descent), GNU lets -H or -L win and walks the directory the link leads to
-hRH "$(id -u):$(id -G | cut -d' ' -f2)" dl
-hRH "$(id -u):$(id -G | cut -d' ' -f2)" ld3
-hRL "$(id -u):$(id -G | cut -d' ' -f2)" dl
-hRL "$(id -u):$(id -G | cut -d' ' -f2)" ld3
-h -R -L "$(id -u):$(id -G | cut -d' ' -f2)" dl
-h -R -L "$(id -u):$(id -G | cut -d' ' -f2)" ld3
-hRH "$(id -u):$(id -G | cut -d' ' -f2)" treelink
-hRH "$(id -u):$(id -G | cut -d' ' -f2)" top3
-hRH "$(id -u):$(id -G | cut -d' ' -f2)" toptop3
-hRL "$(id -u):$(id -G | cut -d' ' -f2)" treelink
-h -R -L "$(id -u):$(id -G | cut -d' ' -f2)" treelink
-hRL "$(id -u):$(id -G | cut -d' ' -f2)" t3
-h -R -L "$(id -u):$(id -G | cut -d' ' -f2)" t3
-hRL "$(id -u):$(id -G | cut -d' ' -f2)" top3
-hRL "$(id -u):$(id -G | cut -d' ' -f2)" toptop3
-h -R -L "$(id -u):$(id -G | cut -d' ' -f2)" top3
-h -R -L "$(id -u):$(id -G | cut -d' ' -f2)" toptop3
# GNU options: -f -v -c --reference --from --no-dereference; the standard has -h -H -L -P -R only, they are usage errors here
-f "$(id -u)" f
-v "$(id -u)" f
-c "$(id -u)" f
--reference=f g
--from=0 "$(id -u)" f
--no-dereference "$(id -u)" sl
