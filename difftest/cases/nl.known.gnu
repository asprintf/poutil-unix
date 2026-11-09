# GNU numbers several files as one; only one file can be named
- text
text text
# GNU restarts the numbering at every section; the spec (and macOS) at each logical page, which starts with the header
-h a -f a -b a pages
-d '\:' -h a pages
-ba -ha -fa -nrz -w4 -s'|' -v7 -i7 pages
-h a -f a pages
-h n -f t -b a pages
-d '\' -h a pages
-b a -h a -f a -n rz -w 4 -s '|' -v 7 -i 7 pages
# GNU takes bytes for -d and a longer argument is not an error; ours reads characters, at most two
-d '😀' -h a emoji
-d abc text
-d 'é€' -h a utf8
-d '😀:' -h a emoji
# GNU takes an empty regular expression for every line; regcomp refuses it on macOS, so here it is an error everywhere
-b p text
