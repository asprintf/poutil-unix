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
