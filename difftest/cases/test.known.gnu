# Three arguments with -a or -o in the middle: the spec says the test is binary ($2 is a binary primary), so
# these join two strings and are true. GNU takes a "--" operand for a unary operator and stops with
# "'--': unary operator expected" (exit 2), although "a -a b" works. Checked: x -a -- , -- -a x, x -o -- also fail.
-- -a --
-- -o ''
