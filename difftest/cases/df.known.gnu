# GNU leaves out file systems in the full listing (the dummy ones, repeated devices) and, on APFS, names the system volume for a file in the Data volume
-P
-kP
-Pk
-k -P
-P -k
-P .
-P d
-P d/sub
-P f
-P p
-P lnd
-P lnf
-P /tmp
-P /private
-P /etc/hosts
-P /Users
-P /Volumes
-P -x /
# the output without -P is unspecified; GNU writes a different layout (its own columns), ours is the -P line
-k
-k -k
@
/
.
d
-- /
/ -k
/ -- /
-k /
-k -- -P /
# -t is a file system type filter there; in the standard it asks for the total space
-t
-t -t
-k -t
-kt
-tk
-t /
# long options are an extension
--help
--version
-P --version
# the standard leaves a special file that is not the device of a mounted file system unspecified; macOS refuses it and so do we, GNU shows the /dev file system
-P /dev/null
-P /dev/null /
-P /dev/tty
-P /dev/zero
# the file systems of several operands: GNU names the system volume for a file in the Data volume
-P . .
-P f d p
# a repeated -P is a full listing: GNU leaves out file systems in it
-P -P
