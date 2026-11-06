# non-ASCII names that collate equal are kept in the order of the directory (see ls.known)
-r .n/u8
# -q: GNU takes U+200B and U+200D as printable, the C library that macOS ls and we use does not
-q .n/q
