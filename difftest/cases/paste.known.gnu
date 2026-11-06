# GNU options the spec does not have: --delimiters --serial --version -z
--delimiters=, p1 p2
--serial p1 p2
--version
-z p1 p2

# no file operand: the synopsis wants one, here it is a usage error; GNU reads standard input
@
in:p1
in:p1 -s
in:p10
in:p10 -s
in:p3
in:p3 -s
in:p7
in:p7 -s
in:p9
in:p9 -s

# an empty -d list: the spec says nothing, here it is a usage error; GNU takes it as no delimiter
-d '' p1 p2
-sd '' p1 p2
