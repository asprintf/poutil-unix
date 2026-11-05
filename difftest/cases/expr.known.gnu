# GNU has no limit on the size of an integer (it uses GMP), we stop at 64 bits and say "overflow"
-9223372036854775808 - 1
-9223372036854775808 / -1
99999999999999999999 + 1
4611686018427387904 '*' 2
# match and length are GNU keywords; the spec leaves them unspecified
match
length = length
# XBD 9.3.5: a range whose end sorts before its start is invalid or matches nothing, unspecified; GNU matches nothing, we refuse
a : '[b-a]'
# we refuse parentheses nested more than a few hundred deep (an error, not a stack overflow); GNU recurses without a limit
$(yes '(' | head -5000) 1 $(yes ')' | head -5000)
# An invalid byte sequence in a UTF-8 locale is undefined for the regular expression; the C library of ours (and macOS) fails the match, GNU matches the valid prefix.
"$(printf 'a\377b')" : '.*'
