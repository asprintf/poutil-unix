# -t f takes the sizes of float, double and long double; GNU also takes 2, which is no size of a C type here.
-t f2 float32
# With no file and the standard input /dev/null an offset operand skips past the end of the input: the standard
# asks for a diagnostic and a non-zero status (ours); GNU says nothing and writes the offset.
+3
+3.
-bc +3
# A directory among the files: ours says it cannot be read and goes on; GNU goes on with null bytes in its place.
-A d dir text
# -t c in a UTF-8 locale: characters are shown as the standard has it (the character, ** for its other bytes, octal
# for what is no character). GNU shows every byte on its own, so bytes of a multibyte character are written raw.
-t c utf8
-t c badutf8
-j 1 -t c utf8
-j 4 -N 3 -t c utf8
-A d -t c utf8
in:utf8 -t c
-t c text utf8
-t c '😀 file.txt'
-A d -t c '😀 file.txt' utf8
-t c bytes256
-bcdosx negative
-b -c -d -o -s -x negative
-xsodcb negative
-t acdfoux bytes256
-t xuofdca bytes256
-t a -t c -t d -t f -t o -t u -t x bytes256
-t x -t u -t o -t f -t d -t c -t a bytes256
# -A n: the offset after the last byte is written as an empty line (the standard says "shall be written", the PCTS
# expects the line, macOS writes it); GNU writes nothing.
-A n text
-v -A n -t x1 zeros
-A n -t x1 -t o1 short
-j 3 -N 4 -A n text
-A n -t a text
# The same for empty input (GNU writes nothing, as for the other bases).
-A n empty
in:empty -A n
