# GNU options the spec does not have: --check-order --nocheck-order --header -i -z -o auto --help --version
--check-order inter.1 inter.2
--header inter.1 inter.2
--help
--nocheck-order inter.1 inter.2
--version
-i case.1 case.2
-o auto -a 1 inter.1 wide.2
-o auto inter.1 inter.2
-z inter.1 inter.2

# a conflicting repeated option: GNU refuses it, macOS and we take the last
-1 1 -1 2 mid.1 mid.2
-2 1 -2 2 mid.1 mid.2
-t '|' -t : pipe.1 pipe.2

# -a with -v is a usage error here (the synopsis has one of them); GNU accepts it
-a1 -v2 inter.1 inter.2

# -t with a multibyte character: GNU refuses it, we take it
-t é -a 1 -a 2 u8.1 u8.2
-t é u8.1 u8.2

# the input is not sorted in this locale or for this -t (unspecified): GNU warns and exits 1
-a 1 -a 2 -o 0,1.2,2.2 -e X case.1 case.2
-a 1 -a 2 case.1 case.2
-a 1 case.1 case.2
-a 2 -e - -o 1.1,2.1,0 case.1 case.2
-a 2 case.1 case.2
-o 0,1.2,2.2 case.1 case.2
-t '	' -1 2 -2 2 tsv.1 tsv.2
-t ' ' -a 1 -a 2 blank.1 blank.2
-t ' ' blank.1 blank.2
-v 1 -o 0,1.2 case.1 case.2
-v 1 -v 2 case.1 case.2
-v 1 case.1 case.2
-v 2 case.1 case.2
case.1 case.2
