# step 1b: the destination names the same directory entry as the source_file, here a symbolic link: a
# diagnostic. GNU follows the link, removes it and makes a hard link to what it led to.
-f sl sl
-f sl2 sl
-fL sl2 sl
-sf sl sl
-sfP sl sl
# the destination was made by the run itself (e to d/e, twice): the spec leaves open an error or going on;
# GNU reports it as well, but with -s it does so only when the second is not a symbolic link of its own
-sf e e d
# one operand: GNU makes the link in the current directory, the synopsis wants two or more
-s -f nl
