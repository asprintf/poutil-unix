# GNU works on bytes, ours on characters (UTF-8)
in:accents '[:lower:]' '[:upper:]'
in:accents '[:upper:]' '[:lower:]'
in:accents '\303\251' e
in:accents 'à-ÿ' '[?*]'
in:accents 'é' e
in:accents 'éè' 'ee'
in:accents -c '[:alpha:]' '_'
in:accents -cd '[:alpha:]'
in:accents -d '[:alpha:]'
in:accents -d '\303\251'
in:accents -d 'é'
in:case '[:lower:]' '[:upper:]'
in:case '[:upper:]' '[:lower:]'
in:cjk -c '\n' x
in:cjk -d '[:alpha:]'
in:emoji '\360\237\230\200' x
in:emoji -c x y
in:emoji x '\360\237\230\200'
in:invalid -c a x
# GNU warns on standard error about a backslash at the end of the string
in:text 'a\' xy
# a repeat count that does not fit in a number: GNU refuses it, here it means as many as string1 needs
in:text abc '[x*18446744073709551615]'
in:text -c a '[x*18446744073709551615]'
in:text abc '[x*99999999999999999999999]'
# GNU's [=e=] is the letter alone; here it is what the C library's [[=e=]] matches, the accented forms of the locale too
in:accents '[=e=]' '[e*]'
in:accents '[=e=]' x
in:accents -ds '[=e=]' x
