# GNU options the spec does not have: -q -v --help --version and -0
--help
--version
-0 ten
-q
-q ten eleven
-v ten

# -c is not in the spec; the last of -c and -n wins here, GNU refuses to have both
-c 3 -n 2 ten
-n 2 -c 3 ten

# GNU accepts a negative count (all but the last N lines)
-n '-1' ten

# GNU puts quotes around a name with a blank or a special character in the header; the spec says %s
'a b' ten
'trailing ' ten
-n 1 '==>' '==>'
-n 1 'a b' ten
-n 1 ten '==>' eleven
-n 100 '==>' '==>'
-n 100 'a b' ten
-n 100 ten '==>' eleven
-n 3 '==>' '==>'
-n 3 'trailing ' ten
-n 3 ten '==>' eleven
