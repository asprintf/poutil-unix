# -i and an answer that is not yes: the file is not copied and that is no error. The spec's EXIT STATUS
# has only "0 successful" and ">0 an error occurred", and the STDERR / -i text says "otherwise, it shall
# not [be attempted]". GNU 9 exits 1 when a prompt is declined, ours (and macOS) exit 0. Every line
# below was checked: GNU's standard error holds nothing but its prompts, the listings are identical.
in:ans.sp_y -i a b
in:ans.sp_y -i -f a b
in:ans.eof -i a b
in:ans.eof -i -f a b
in:ans.one -i a b
in:ans.one -i -f a b
in:ans.yn -i a b c tg2/
in:ans.yn -R -i d/. mg
in:ans.yn -f -i a b c tg2
in:ans.ny -i a b c tg2
in:ans.ny -i -p a b c tg2
in:ans.ny -i -f a b c tg2
in:ans.yy -i a b c tg2/
in:ans.yy -f -i a b c tg2
in:ans.nn -i a b c tg2
in:ans.nn -i -p a b c tg2
in:ans.nn -i -f a b c tg2
in:ans.ynyn -i a b c tg2/
in:ans.ynyn -R -i d/. mg
in:ans.ynyn -f -i a b c tg2
in:ans.nyy -i a b c tg2
in:ans.nyy -i -p a b c tg2
in:ans.nyy -i -f a b c tg2
in:ans.y -i a b c tg2/
in:ans.y -R -i d/. mg
in:ans.y -f -i a b c tg2
in:ans.n -i a b c tg2
in:ans.n -i -p a b c tg2
in:ans.n -i -f a b c tg2
in:ans.eof -i a b c tg2/
in:ans.eof -R -i d/. mg
in:ans.eof -f -i a b c tg2
in:ans.n -i a rof
in:ans.n -i a b
in:ans.n -i -f a b
in:ans.no -i a b
in:ans.no -i -f a b
in:ans.blank -i a b
in:ans.blank -i -f a b
in:ans.emoji -i a b
in:ans.emoji -i -f a b
in:ans.zero -i a b
in:ans.zero -i -f a b
in:ans.yn -i a b c tg2
in:ans.yn -i -p a b c tg2
in:ans.yn -i -f a b c tg2
in:ans.ny -i a b c tg2/
in:ans.ny -R -i d/. mg
in:ans.ny -f -i a b c tg2
in:ans.yy -i a b c tg2
in:ans.yy -i -p a b c tg2
in:ans.yy -i -f a b c tg2
in:ans.nn -i a b c tg2/
in:ans.nn -R -i d/. mg
in:ans.nn -f -i a b c tg2
in:ans.ynyn -i a b c tg2
in:ans.ynyn -i -p a b c tg2
in:ans.ynyn -i -f a b c tg2
in:ans.nyy -i a b c tg2/
in:ans.nyy -R -i d/. mg
in:ans.nyy -f -i a b c tg2
in:ans.y -i a b c tg2
in:ans.y -i -p a b c tg2
in:ans.y -i -f a b c tg2
in:ans.n -i a b c tg2/
in:ans.n -R -i d/. mg
in:ans.n -f -i a b c tg2
in:ans.eof -i a b c tg2
in:ans.eof -i -p a b c tg2
in:ans.eof -i -f a b c tg2
in:ans.n -P -i la b
in:ans.n -if a rof
in:ans.n -i -p pa pb
in:ans.n -ii a b
# GNU's own warning "source file specified more than once"; copying the same file twice is no error
# for the spec and ours says nothing. (Listings and status are the same.)
a a b tgt
-R ff640 ff640 tgt
