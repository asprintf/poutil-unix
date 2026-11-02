# an empty string for %c: nothing or a null byte is written (item 11); GNU writes the null byte, we write nothing
'[%c]\n' ''
'[%c]\n'
'[%s][%b][%c][%d][%i][%u][%o][%x][%X]\n'
# a precision for %b is the number of bytes written (item 7); GNU refuses it
'%.2b|\n' 'a\tb'
'%.0b|\n' abc
# a format with no conversion and arguments: the results are unspecified (item 9); GNU warns about the excess arguments, we ignore them without a message
'hello\n' a b
'100%%\n' a b
# a field width or precision too big for an int: we say so and stop, GNU does something else (unspecified)
'%99999999999d\n' 1
'%.99999999999s\n' 1
# GNU takes an operand that starts with - as the format; printf has no options, so here it is a usage error unless -- comes first
'-%d\n' 5
-x
-d
# length modifiers and %q are not in the conversion specifications of the standard
'%ld\n' 5
'%lld\n' 5
'%hd\n' 5
'%q\n'
# GNU converts the argument of a floating-point conversion as a long double, not as strtod() does: more digits for a big number, and no overflow below 1e4932
'%a\n' 1e300
'%a\n' 3.14159
'%f\n' 1e300
'%f\n' 1e999
'%f\n' -1e999
# macOS and GNU report the underflow of 1e-999 (GNU: a different digit string); the standard names overflow (APPLICATION USAGE), and a number too small to hold is not an error here
'%f|%e|\n' 1e-999 1e-999
# the rounding of a hexadecimal floating-point number with no digits after the point is the C library's choice
'%.0a\n' 1.5
# length modifiers are not in the conversion specifications of the standard
'%lf\n' 1.5
'%lg\n' 1.5
