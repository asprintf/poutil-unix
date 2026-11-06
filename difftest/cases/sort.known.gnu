# GNU fails in strcoll for -d -i and copied keys; -f folds single bytes only
-C u8.invalid
-d u8.accents
-d u8.sharp
-f u8.turk
-fu u8.accents
-fu u8.sharp
-fu u8.turk
-i u8.accents
-i u8.sharp
-k 1.1,1.1 u8.accents
-k 1.2 u8.accents
-k 1.2,1.2 u8.sharp
-r u8.combining
in:u8.accents -fu

# -c -C -m -o do not combine and -c takes one file (synopsis); the references differ
-c -C sorted
-C -c sorted
-c -C unsorted
-C -c unsorted
-c -m
-cC unsorted
-Cc unsorted
-cm m1
-Cm sorted
-cum m1
-mc m1

# a repeated -t or -o: GNU refuses differing ones; macOS and we take the last
-o out -o out2 words
-o out2 -o out words
-t : -t , -k 2 sep.colon
-t a -t b words

# -d with -i or -n is undefined; GNU refuses it
-k 1,1bfdinr mixed

# -y and a signed -k: macOS refuses, GNU takes them
-k +1 words
-y words

