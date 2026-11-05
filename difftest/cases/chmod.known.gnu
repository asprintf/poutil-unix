# For a directory it is implementation-defined whether a request to set or clear the set-user-ID or
# set-group-ID bit is honored (an octal mode sets the bits "absolutely", "=" clears them): ours does what
# is asked, GNU keeps the bits of a directory unless they are named (and clears the sticky bit of a
# directory when "=" names the class "o"). Only the directories differ in these lines.
0 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
644 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
755 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
1755 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
4755 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
=g f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
=o f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
=r f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
=rwx f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
=t f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
=u f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
=x f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
=X f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a= f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=,a+t f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=,u+r f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=g f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=o f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=r f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=rwx,a-w,a-r f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=rX f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=t f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=u f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=X f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a+r-x=u f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
g= f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
g=u f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
gu=rw f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
o= f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
o=g,g=u f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
o=u f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
o=X f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
u+X,g=u f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
u=o,g=u,o=g f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
u=rwx,g=rx,o=r f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
u=rwx,g=u f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
u=rwx,go= f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
u=X,g=u f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ug= f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
# The perm t with the who o, g or u: "not an error, but the meaning ... is unspecified". Ours leaves the
# bits alone, GNU sets the sticky bit (of files too).
o+t f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
-R o+t dt dg
# The perm X means the execute bit of a directory, or of a file whose "current (unmodified)" mode has
# one. GNU looks at the mode as the clauses before have left it.
a-x,a+X f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=,a+X f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=r,a+X f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=x,a+X f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
g=,a+X f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
# A directory whose read permission a mode takes away is changed after what is in it, or before it is
# read if it cannot be read as it is: every file is changed and there is no error. GNU changes the
# directory first and then fails to read it.
-R 0 d7
-R 000 tree
-R a-r tree
-R a-x tree
-R u-x tree
# "-1" and "+1": GNU takes them for a mode; they are an option and an invalid mode here.
-1 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
+1 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
# The set-id and sticky bits where the spec leaves the result open (vetted: in every line below the
# results differ only in the s, S, t and T positions of the mode): for a directory it is
# implementation-defined whether a request to set or clear set-user-ID or set-group-ID is honored; the perm
# symbol t is unspecified with who u, g or o and with any file that is no directory (GNU sets it, we
# leave it); X reads the "current (unmodified)" mode, which we take as the mode before the whole
# command (so a later a-X after a=rwxXst ignores X) and GNU as the mode so far.
= f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
=w f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
g=r f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
g=w f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
g=x f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
g=X f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
g=t f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
g=rwx f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
g=g f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
g=o f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
o+rwxXst f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
o-t f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
o-rwxXst f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
o=r f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
o=w f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
o=x f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
o=s f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
o=t f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
o=rwx f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
o=rwxXst f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
o=g f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
o=o f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=w f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=x f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=rwx f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ug=r f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ug=w f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ug=x f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ug=X f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ug=t f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ug=rwx f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ug=u f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ug=g f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ug=o f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ugo= f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ugo=r f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ugo=w f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ugo=x f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ugo=X f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ugo=t f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ugo=rwx f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ugo=u f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ugo=g f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ugo=o f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=r+w-x f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
u=rwx,g=u,o= f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
ug=rw,o=r f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
u+s,o+t f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
u-s,g-s,o-t f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=rwxXst,a-X f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
u=,g=,o= f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
=rwx,u-w f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
u=rwx,g=rx,o=rx,a-w f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=rx,u+w f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
a=r,a+w,a+x f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
00 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
000 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
0000 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
7 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
07 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
77 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
077 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
777 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
0777 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
1777 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
4777 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
0644 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
600 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
400 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
200 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
100 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
1 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
2 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
4 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
1000 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
4000 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
0755 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
== f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
# GNU takes a number after an operator ("+644", "=644", "-1", "+1") as octal bits; the spec has octal only
# alone and symbolic only with letters; ours refuses it
-- -1 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
# Options the spec does not have (-h, -L, -H, -P, and an unknown one after -R): ours is a usage error; macOS and GNU accept them (macOS reads -Rx as a symbolic mode).
-h 600 l
-L 600 l
-RH 600 tree2link
-RL 600 tree2link
-RP 600 tree2link
# +644, =644 and +1 are no symbolic mode and no octal mode; GNU takes them for a mode, ours is an invalid mode (as macOS).
+1 f6
+644 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
+644 f6
=644 f0 f4 f6 f7 f77 fs fg d0 d7 dt dg
=644 f6
# -R: a link operand or a link met is not followed here for macOS, a dangling one is no error there, and a directory that cannot be read is changed first and then fails; see above.
-R 000 'dir 😀'
-R a= tree2/s tree2/a
