# GNU takes a sign or a digit after an option letter (+1, -c1)
-c1 f.keys
-c1 plain
-f +1 f.keys
-s +1 s.short

# GNU also counts U+00A0, U+0085, U+2003 and U+3000 as blanks for -f; we use iswblank (the locale decides)
-c -f 1 f.u8blank
-f 1 f.u8blank
in:f.u8blank -f 1

# GNU options the spec does not have: -w -z --help --version
--help
--version
-w 1 plain
-z plain

# GNU cannot open /dev/stdout for the output file
-c plain /dev/stdout
plain /dev/stdout
