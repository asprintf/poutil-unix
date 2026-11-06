# GNU options the spec does not have: -z --check-order --nocheck-order --output-delimiter --total --help --version
--check-order interleave.1 interleave.2
--help
--nocheck-order interleave.1 interleave.2
--output-delimiter=: interleave.1 interleave.2
--total interleave.1 interleave.2
--version
-z interleave.1 interleave.2

# the input is not sorted in this locale (the spec leaves the result open): GNU warns and exits 1
-1 case.1 case.2
-1 collate.1 collate.2
-1 invalid.1 invalid.2
-12 case.1 case.2
-12 case.2 case.1
-12 collate.1 collate.2
-12 invalid.1 invalid.2
-12 unsorted.1 interleave.1
-12 unsorted.1 unsorted.2
-123 case.1 case.2
-123 case.2 case.1
-123 collate.1 collate.2
-123 collate2.1 collate2.2
-123 invalid.1 invalid.2
-123 utf8.1 utf8.2
-13 case.1 case.2
-13 collate.1 collate.2
-13 invalid.1 invalid.2
-2 case.1 case.2
-2 collate.1 collate.2
-2 invalid.1 invalid.2
-23 case.1 case.2
-23 collate.1 collate.2
-23 invalid.1 invalid.2
-3 case.1 case.2
-3 case.2 case.1
-3 collate.1 collate.2
-3 invalid.1 invalid.2
-3 unsorted.1 unsorted.2
-3 unsorted.2 unsorted.1
case.1 case.2
case.2 case.1
collate.1 collate.2
in:collate.1 - collate.2
in:invalid.1 - invalid.2
interleave.1 unsorted.2
invalid.1 invalid.2
unsorted.1 unsorted.2
