#!/bin/sh
# regress.sh - regression tests that need a real process.
#
# Usage: sh difftest/regress.sh
#
# difftest.sh compares plain argument lists with other implementations and
# cargo test checks the pure cores. This script covers what neither can
# express: redirections (cat f >> f), closed descriptors, SIGPIPE, resource
# limits, signals sent to real processes, umask and inherited dispositions.
# Every check has a fixed expectation; no reference tool is needed beyond the
# system's own sh, ls, od, cmp and sleep.
#
# A writer that loops (the cat f >> f bug) ends with SIGXFSZ because of the
# file size limit below, instead of filling the disk.
#
# Environment: OURS (default target/release)
# Exit status: 0 if every check passed, 1 otherwise.

here=$(cd "$(dirname "$0")" && pwd)
OURS=${OURS:-$here/../target/release}
OURS=$(cd "$OURS" && pwd) || exit 1
LC_ALL=C
export LC_ALL
umask 022
ulimit -f 40960 2>/dev/null

tmp=$(mktemp -d "${TMPDIR:-/tmp}/regress.XXXXXX") || exit 1
trap 'chmod -R u+rwx "$tmp" 2>/dev/null; rm -rf "$tmp"' EXIT
trap 'exit 1' HUP INT TERM

pass=0
fail=0

# eq NAME WANT GOT
eq() {
	if [ "$2" = "$3" ]; then
		pass=$((pass + 1))
	else
		fail=$((fail + 1))
		printf 'FAIL %s\n  want: [%s]\n  got:  [%s]\n' "$1" \
			"$(printf %s "$2" | head -c 200)" "$(printf %s "$3" | head -c 200)"
	fi
}

# fresh: an empty working directory
fresh() {
	chmod -R u+rwx "$tmp/w" 2>/dev/null
	rm -rf "$tmp/w"
	mkdir "$tmp/w"
	cd "$tmp/w" || exit 1
}

mode() {
	ls -ld "$1" | cut -c1-10
}

# hex FILE_OR_STDIN: bytes as a run of hex digits
hex() {
	od -An -tx1 | tr -d ' \n'
}

# reap PID: wait for a background process, hiding the shell's job message
reap() {
	{ wait "$1"; } 2>/dev/null
}

#---------------------------------------------------------------------- cat
fresh
echo hi > o
"$OURS/cat" o >> o 2> err
eq "cat o >> o: status" 1 $?
eq "cat o >> o: file untouched" hi "$(cat o)"
eq "cat o >> o: message" "cat: o: input file is output file" "$(cat err)"

"$OURS/cat" o o >> o 2> err
eq "cat o o >> o: status" 1 $?
eq "cat o o >> o: file untouched" hi "$(cat o)"
eq "cat o o >> o: both refused" 2 "$(wc -l < err | tr -d ' ')"

echo x > p
"$OURS/cat" p o >> o 2> /dev/null
eq "cat p o >> o: status" 1 $?
eq "cat p o >> o: the other input still copied" "hi x" "$(cat o | tr '\n' ' ' | sed 's/ $//')"

echo hi > o
"$OURS/cat" - >> o < o 2> err
eq "cat - >> o < o: status" 1 $?
eq "cat - >> o < o: message" "cat: -: input file is output file" "$(cat err)"
eq "cat - >> o < o: file untouched" hi "$(cat o)"

"$OURS/cat" o > o
eq "cat o > o: status" 0 $?
eq "cat o > o: file was truncated first" 0 "$(wc -c < o | tr -d ' ')"

echo hi > o
"$OURS/cat" o > p
eq "cat o > p: status" 0 $?
eq "cat o > p: copied" hi "$(cat p)"

head -c 5000000 /dev/urandom > big
"$OURS/cat" big > big.out
eq "cat 5MB: status" 0 $?
cmp -s big big.out
eq "cat 5MB: identical" 0 $?
"$OURS/cat" < big > big.out2
cmp -s big big.out2
eq "cat 5MB from stdin: identical" 0 $?

{ "$OURS/cat" big; echo $? > st; } | head -c 10 > /dev/null
eq "cat into a closed pipe dies of SIGPIPE" 141 "$(cat st)"

echo data > f1
"$OURS/cat" f1 >&- 2> err
eq "cat with stdout closed: status" 1 $?
eq "cat with stdout closed: diagnostic" 1 "$(wc -l < err | tr -d ' ')"
"$OURS/cat" <&- 2> err
eq "cat with stdin closed: status" 1 $?
"$OURS/cat" . 2> /dev/null
eq "cat of a directory: status" 1 $?

out=$("$OURS/cat" nosuch f1 2> /dev/null)
eq "cat nosuch f1: later files still read" data "$out"
"$OURS/cat" nosuch f1 > /dev/null 2>&1
eq "cat nosuch f1: status" 1 $?

eq "cat - -: one stdin" x "$(printf x | "$OURS/cat" - -)"

mkfifo ff
echo fifo-data > ff &
eq "cat of a fifo" fifo-data "$("$OURS/cat" ff)"
wait

# the output may be reached by another name: hard link, symlink, stdin
fresh
echo hi > o
ln o hard
ln -s o soft
: > e
"$OURS/cat" o >> hard 2> /dev/null
eq "cat o >> hard link of o: refused" "1 hi" "$? $(cat o)"
"$OURS/cat" o >> soft 2> /dev/null
eq "cat o >> symlink to o: refused" "1 hi" "$? $(cat o)"
"$OURS/cat" soft >> o 2> /dev/null
eq "cat symlink >> o: refused" "1 hi" "$? $(cat o)"
"$OURS/cat" < o >> soft 2> /dev/null
eq "cat < o >> symlink to o: refused" "1 hi" "$? $(cat o)"
"$OURS/cat" e >> e
eq "cat of an empty file onto itself: nothing to copy, no error" "0 0" "$? $(wc -c < e | tr -d ' ')"

# unreadable input, input that is a directory on stdin, shared offsets
echo x > unr
chmod 000 unr
echo y > ok
"$OURS/cat" unr ok > out 2> err
eq "cat unreadable ok: status" 1 $?
eq "cat unreadable ok: the readable file is copied" y "$(cat out)"
eq "cat unreadable ok: one diagnostic" 1 "$(wc -l < err | tr -d ' ')"
chmod 644 unr
"$OURS/cat" < . > out 2> err
eq "cat < directory: status" 1 $?
eq "cat < directory: nothing copied" 0 "$(wc -c < out | tr -d ' ')"
printf '0123456789' > ten
eq "cat continues where the shared offset is" 456789 "$({ dd bs=1 count=4 of=/dev/null 2> /dev/null; "$OURS/cat"; } < ten)"
eq "cat of an endless file stops when the reader is gone" 100000 "$("$OURS/cat" /dev/zero | head -c 100000 | wc -c | tr -d ' ')"
printf a > na
printf b > nb
printf 'a\0b' > nul
eq "cat: files without a final newline are joined as they are" 6162 "$("$OURS/cat" na nb | hex)"
eq "cat of a file with a NUL byte" 610062 "$("$OURS/cat" nul | hex)"

# data goes out as it arrives, with and without -u
t0=$(date +%s)
{ echo first; sleep 2; echo second; } | "$OURS/cat" | { read l1; t1=$(date +%s); read l2; echo "$l1 $l2 $((t1 - t0))" > lat1; } &
{ echo first; sleep 2; echo second; } | "$OURS/cat" -u | { read l1; t1=$(date +%s); read l2; echo "$l1 $l2 $((t1 - t0))" > lat2; } &
wait
eq "cat: the first line is passed on before the writer is done" ok "$(awk '{ print ($1 == "first" && $2 == "second" && $3 <= 1) ? "ok" : "bad " $0 }' lat1)"
eq "cat -u: the first line is passed on before the writer is done" ok "$(awk '{ print ($1 == "first" && $2 == "second" && $3 <= 1) ? "ok" : "bad " $0 }' lat2)"

#--------------------------------------------------------------------- echo
fresh
eq "echo: no operand is a newline" 0a "$("$OURS/echo" | hex)"
eq "echo a b: one blank between, a newline after" 6120620a "$("$OURS/echo" a b | hex)"
eq "echo -n a: -n is a string under XSI" 2d6e20610a "$("$OURS/echo" -n a | hex)"
eq "echo -- a: so is --" 2d2d20610a "$("$OURS/echo" -- a | hex)"
eq "echo: \\a \\b \\f \\r \\t \\v" 07080c0d090b0a "$("$OURS/echo" '\a\b\f\r\t\v' | hex)"
eq "echo: \\n" 0a0a "$("$OURS/echo" '\n' | hex)"
eq "echo: \\\\" 5c0a "$("$OURS/echo" '\\' | hex)"
eq "echo: \\0101 is A" 410a "$("$OURS/echo" '\0101' | hex)"
eq "echo: \\0 is a NUL byte" 000a "$("$OURS/echo" '\0' | hex)"
eq "echo: \\0 takes at most three more digits" 41310a "$("$OURS/echo" '\01011' | hex)"
eq "echo: \\c stops everything, the newline too" 61 "$("$OURS/echo" 'a\cb' c | hex)"
eq "echo: \\c in a later operand keeps what came before" 6120 "$("$OURS/echo" a '\c' b | hex)"
eq "echo: \\c alone prints nothing" "" "$("$OURS/echo" '\c' | hex)"
eq "echo: an unknown escape does not fail" 0 "$("$OURS/echo" '\q' > /dev/null; echo $?)"
eq "echo: bytes that are not characters pass through" ff80780a "$("$OURS/echo" "$(printf '\377\200x')" | hex)"
big=$(awk 'BEGIN { s = sprintf("%100000s", ""); gsub(/ /, "x", s); printf "%s", s }')
eq "echo: a 100000-byte operand" 100001 "$("$OURS/echo" "$big" | wc -c | tr -d ' ')"
eq "echo: 20000 operands" "$(seq 1 20000 | tr '\n' ' ' | sed 's/ $//')" "$("$OURS/echo" $(seq 1 20000))"
"$OURS/echo" hi <&- > /dev/null
eq "echo with stdin closed: status" 0 $?
"$OURS/echo" hi >&- 2> err
eq "echo with stdout closed: status" 1 $?
eq "echo with stdout closed: one diagnostic" 1 "$(wc -l < err | tr -d ' ')"
{ "$OURS/echo" $(seq 1 50000); echo $? > st; } | head -c 10 > /dev/null
eq "echo into a closed pipe dies of SIGPIPE" 141 "$(cat st)"
if [ "$(LC_ALL=en_US.UTF-8 locale charmap 2> /dev/null)" = UTF-8 ]; then
	E=$(printf '\303\251')
	eq "echo: a multibyte character passes through" "c3a90a" "$(LC_ALL=en_US.UTF-8 "$OURS/echo" "$E" | hex)"
	eq "echo: an escape next to a multibyte character" "c3a9090a" "$(LC_ALL=en_US.UTF-8 "$OURS/echo" "$E\\t" | hex)"
fi

#---------------------------------------------------------------------- tee
fresh
echo x | "$OURS/tee" a a > /dev/null
eq "tee a a: status" 0 $?
eq "tee a a: one copy" x "$(cat a)"

echo y | "$OURS/tee" -a a > /dev/null
eq "tee -a: appends" "x y" "$(cat a | tr '\n' ' ' | sed 's/ $//')"

out=$(echo x | "$OURS/tee" /nonexistent/f 2> /dev/null)
eq "tee bad file: stdout still written" x "$out"
echo x | "$OURS/tee" /nonexistent/f > /dev/null 2>&1
eq "tee bad file: status" 1 $?

head -c 3000000 /dev/urandom > src
"$OURS/tee" t1 < src > t1.out
cmp -s src t1
eq "tee 3MB: file copy identical" 0 $?
cmp -s src t1.out
eq "tee 3MB: stdout copy identical" 0 $?

{ "$OURS/tee" t2 < src; echo $? > st; } | head -c 10 > /dev/null
eq "tee into a closed pipe dies of SIGPIPE" 141 "$(cat st)"

files=$(seq 1 400 | sed 's/^/m/')
echo x | "$OURS/tee" $files > /dev/null
eq "tee 400 files: status" 0 $?
eq "tee 400 files: all created" 400 "$(ls m* | wc -l | tr -d ' ')"
eq "tee 400 files: all written" 1 "$(cat m* | sort -u | wc -l | tr -d ' ')"
(
	ulimit -n 20
	echo x | "$OURS/tee" $files > out 2> err
	echo $? > st
)
eq "tee out of descriptors: status" 1 "$(cat st)"
eq "tee out of descriptors: stdout still written" x "$(cat out)"
eq "tee out of descriptors: diagnostics" yes "$([ "$(wc -l < err | tr -d ' ')" -gt 0 ] && echo yes)"

# an unwritable file is reported, the other files and stdout still get the data
fresh
echo old > ro
chmod 444 ro
echo x | "$OURS/tee" ro g > out 2> err
eq "tee read-only file: status" 1 $?
eq "tee read-only file: stdout written" x "$(cat out)"
eq "tee read-only file: the next file written" x "$(cat g)"
eq "tee read-only file: it stays as it was" old "$(cat ro)"
eq "tee read-only file: one diagnostic" 1 "$(wc -l < err | tr -d ' ')"
echo x | "$OURS/tee" /dev/null > out
eq "tee /dev/null: stdout written" x "$(cat out)"
printf 'no newline' | "$OURS/tee" nn > out
eq "tee: a last line without newline stays so" "no newline|no newline" "$(cat nn)|$(cat out)"
: | "$OURS/tee" empty > out
eq "tee of empty input: empty file made" "0 0" "$(wc -c < empty | tr -d ' ') $(wc -c < out | tr -d ' ')"
printf 'longer old contents\n' > tr
echo s | "$OURS/tee" tr > /dev/null
eq "tee truncates an existing file" s "$(cat tr)"
printf 'a\n' > ap
printf 'b' | "$OURS/tee" -a ap > /dev/null
printf 'c\n' | "$OURS/tee" -a ap > /dev/null
eq "tee -a twice: appends every time, no newline added" "a bc" "$(cat ap | tr '\n' ' ' | sed 's/ $//')"

# -i: SIGINT is ignored; without it the default action applies. A background
# job of this sh ignores SIGINT already, so perl puts the default back first.
# (a script, not a shell function: $! must be the pid of tee itself)
cat > "$tmp/dfl_int" <<'EOF2'
#!/usr/bin/perl
$SIG{INT} = "DEFAULT";
exec @ARGV;
EOF2
chmod 755 "$tmp/dfl_int"
{ echo first; sleep 2; echo second; } | "$tmp/dfl_int" "$OURS/tee" -i ti > ti.out &
p1=$!
{ echo first; sleep 2; echo second; } | "$tmp/dfl_int" "$OURS/tee" tn > tn.out &
p2=$!
sleep 1
kill -INT $p1 $p2
reap $p1
eq "tee -i survives SIGINT: status" 0 $?
reap $p2
eq "tee without -i dies of SIGINT: status" 130 $?
eq "tee -i survives SIGINT: the file has everything" "first second" "$(tr '\n' ' ' < ti | sed 's/ $//')"
eq "tee -i survives SIGINT: so does stdout" "first second" "$(tr '\n' ' ' < ti.out | sed 's/ $//')"
eq "tee without -i: only what came before the signal" first "$(cat tn)"

#------------------------------------------------------- mkdir mkfifo rmdir
fresh
"$OURS/mkdir" -pm 700 g1/g2
eq "mkdir -pm 700: leaf" drwx------ "$(mode g1/g2)"
eq "mkdir -pm 700: parent follows the umask" drwxr-xr-x "$(mode g1)"
"$OURS/mkdir" -m a+x,u-w w
eq "mkdir -m a+x,u-w: relative to a=rwx" dr-xrwxrwx "$(mode w)"
"$OURS/mkdir" g6 -p
eq "mkdir g6 -p: options after an operand are operands" "both made" "$([ -d g6 ] && [ -d ./-p ] && echo both made)"
(
	umask 777
	"$OURS/mkdir" -p x/y
)
eq "mkdir -p, umask 777: parent still usable" d-wx------ "$(mode x)"
eq "mkdir -p, umask 777: leaf follows the umask" d--------- "$(mode x/y)"
(
	umask 077
	"$OURS/mkdir" -p um/a/b
)
eq "mkdir -p, umask 077: every level" "drwx------ drwx------ drwx------" "$(mode um) $(mode um/a) $(mode um/a/b)"
"$OURS/mkdir" -p um/a/b
eq "mkdir -p twice: status" 0 $?
"$OURS/mkdir" um 2> /dev/null
eq "mkdir existing: status" 1 $?
touch file
"$OURS/mkdir" -p file/x 2> /dev/null
eq "mkdir -p under a file: status" 1 $?
ln -s um lnk
"$OURS/mkdir" -p lnk 2> /dev/null
eq "mkdir -p of a symlink to a directory: status" 0 $?
ln -s nowhere dang
"$OURS/mkdir" -p dang 2> /dev/null
eq "mkdir -p of a dangling symlink: status" 1 $?

"$OURS/mkfifo" -m 600 f1
eq "mkfifo -m 600" prw------- "$(mode f1)"
"$OURS/mkfifo" -m u+x f2
eq "mkfifo -m u+x: relative to a=rw" prwxrw-rw- "$(mode f2)"
"$OURS/mkfifo" -m 4755 f3
eq "mkfifo -m 4755: setuid kept" prwsr-xr-x "$(mode f3)"
(
	umask 077
	"$OURS/mkfifo" f4
)
eq "mkfifo under umask 077" prw------- "$(mode f4)"
"$OURS/mkfifo" f1 2> /dev/null
eq "mkfifo existing: status" 1 $?
"$OURS/mkfifo" -m 0 f5 f6
eq "mkfifo -m 0 f5 f6: both made" "p--------- p---------" "$(mode f5) $(mode f6)"
"$OURS/mkfifo" f7
eq "mkfifo without -m under umask 022: a=rw minus umask" prw-r--r-- "$(mode f7)"
"$OURS/mkfifo" f1 f8 2> /dev/null
eq "mkfifo existing then new: status" 1 $?
eq "mkfifo existing then new: the later operand is still made" p "$(mode f8 | cut -c1)"
"$OURS/mkfifo" f9 f9 2> /dev/null
eq "mkfifo f9 f9: second fails" 1 $?
"$OURS/mkfifo" rt
echo through > rt &
eq "mkfifo: it is a pipe and it carries data" "pipe through" "$([ -p rt ] && echo pipe) $(cat rt)"
wait
mkdir ro
chmod 555 ro
"$OURS/mkfifo" ro/f 2> /dev/null
eq "mkfifo in a read-only directory: status" 1 $?
eq "mkfifo in a read-only directory: nothing made" "" "$(ls ro)"
chmod 755 ro

mkdir -p a/b/c
touch a/keep
"$OURS/rmdir" -p a/b/c 2> /dev/null
eq "rmdir -p stops at a non-empty parent: status" 1 $?
eq "rmdir -p stops at a non-empty parent: what is left" "keep" "$(ls a)"
mkdir -p r1/r2/r3
"$OURS/rmdir" -p r1/r2/r3
eq "rmdir -p removes the whole chain" "0 gone" "$? $([ -e r1 ] || echo gone)"
mkdir -p t1/t2
"$OURS/rmdir" -p t1/t2/
eq "rmdir -p with a trailing slash" "0 gone" "$? $([ -e t1 ] || echo gone)"
mkdir -p u1/u2
"$OURS/rmdir" -p ./u1/u2 2> /dev/null
eq "rmdir -p ./u1/u2: everything below . goes, . itself fails" "1 gone" "$? $([ -e u1 ] || echo gone)"
mkdir d1
ln -s d1 dl
"$OURS/rmdir" dl 2> /dev/null
eq "rmdir of a symlink: refused" "1 kept" "$? $([ -d d1 ] && echo kept)"

#------------------------------------------------------------- pwd
fresh
W=$(/bin/pwd -P)
mkdir -p real/sub
ln -s real/sub link
eq "pwd -L through PWD" "$W/link" "$(cd link && env PWD="$W/link" "$OURS/pwd" -L)"
eq "pwd default is -L" "$W/link" "$(cd link && env PWD="$W/link" "$OURS/pwd")"
eq "pwd -P resolves" "$W/real/sub" "$(cd link && env PWD="$W/link" "$OURS/pwd" -P)"
eq "pwd -LP: last wins" "$W/real/sub" "$(cd link && env PWD="$W/link" "$OURS/pwd" -LP)"
eq "pwd -PL: last wins" "$W/link" "$(cd link && env PWD="$W/link" "$OURS/pwd" -PL)"
eq "pwd -L, PWD unrelated" "$W/real/sub" "$(cd link && env PWD=/tmp "$OURS/pwd" -L)"
eq "pwd -L, PWD relative" "$W/real/sub" "$(cd link && env PWD=./link "$OURS/pwd" -L)"
eq "pwd -L, PWD with a dot component" "$W/real/sub" "$(cd link && env PWD="$W/link/." "$OURS/pwd" -L)"
eq "pwd -L, PWD with a dot-dot component" "$W/real/sub" "$(cd link && env PWD="$W/real/../link" "$OURS/pwd" -L)"
eq "pwd -L, PWD unset" "$W/real/sub" "$(cd link && env -u PWD "$OURS/pwd" -L)"
mkdir gone
(
	cd gone && rmdir ../gone
	"$OURS/pwd" > /dev/null 2>&1
	echo $? > ../st
)
eq "pwd in a removed directory: status" 1 "$(cat st)"

#------------------------------------------------------- link unlink
fresh
echo a > a
"$OURS/link" a b
eq "link: status" 0 $?
eq "link: same inode" "$(ls -i a | awk '{print $1}')" "$(ls -i b | awk '{print $1}')"
echo other > c
"$OURS/link" a c 2> /dev/null
eq "link onto an existing file: status" 1 $?
eq "link onto an existing file: target untouched" other "$(cat c)"
mkdir d
"$OURS/link" d e 2> /dev/null
eq "link of a directory: status" 1 $?
eq "link of a directory: nothing made" "" "$([ -e e ] && echo made)"
"$OURS/link" a a 2> /dev/null
eq "link a a: status" 1 $?

ln -s a sl
"$OURS/unlink" sl
eq "unlink of a symlink: status" 0 $?
eq "unlink of a symlink: link gone, target kept" "gone kept" "$([ -L sl ] || echo gone) $([ -f a ] && echo kept)"
"$OURS/unlink" d 2> /dev/null
eq "unlink of a directory: refused" "1 kept" "$? $([ -d d ] && echo kept)"
"$OURS/unlink" b
"$OURS/unlink" b 2> /dev/null
eq "unlink twice: second fails" 1 $?

# hard links share the data and the link count, unlink drops one name
fresh
echo data > h1
"$OURS/link" h1 h2
"$OURS/link" h2 h3
eq "link: three names, link count 3" 3 "$(ls -l h1 | awk '{print $2}')"
echo more >> h3
eq "link: a write through one name shows through the others" "data more" "$(cat h1 | tr '\n' ' ' | sed 's/ $//')"
"$OURS/unlink" h1
eq "unlink: the count drops and the data stays" "2 data more" "$(ls -l h2 | awk '{print $2}') $(cat h2 | tr '\n' ' ' | sed 's/ $//')"
mkdir sub
"$OURS/link" h2 sub/../h4
eq "link through a .. component: same inode" "$(ls -i h2 | awk '{print $1}')" "$(ls -i h4 | awk '{print $1}')"
"$OURS/link" nosuch h5 2> /dev/null
eq "link of a missing file: status" 1 $?
eq "link of a missing file: nothing made" "" "$([ -e h5 ] && echo made)"
"$OURS/link" h2 nodir/h6 2> /dev/null
eq "link into a missing directory: status" 1 $?
mkdir ro
chmod 555 ro
"$OURS/link" h2 ro/h7 2> /dev/null
eq "link into a read-only directory: status" 1 $?
eq "link into a read-only directory: nothing made" "" "$(ls ro)"
chmod 755 ro
"$OURS/link" h2 ro/h8
chmod 555 ro
"$OURS/unlink" ro/h8 2> /dev/null
eq "unlink in a read-only directory: refused, the name stays" "1 h8" "$? $(ls ro)"
chmod 755 ro
ln -s nowhere dang
"$OURS/unlink" dang
eq "unlink of a dangling symlink" "0 gone" "$? $([ -L dang ] || echo gone)"
"$OURS/link" h2 h9 > out 2> err
eq "link is silent when it succeeds" "0 0" "$(wc -c < out | tr -d ' ') $(wc -c < err | tr -d ' ')"
"$OURS/unlink" h9 > out 2> err
eq "unlink is silent when it succeeds" "0 0" "$(wc -c < out | tr -d ' ') $(wc -c < err | tr -d ' ')"

#------------------------------------------------------------- kill
fresh
sleep 30 < /dev/null > /dev/null 2>&1 &
p=$!
"$OURS/kill" -s 0 $p
eq "kill -s 0: process exists" 0 $?
"$OURS/kill" -s 0 $p
eq "kill -s 0: and is not harmed" 0 $?
"$OURS/kill" -s NOSUCH $p 2> /dev/null
eq "kill -s NOSUCH: refused" 2 $?
"$OURS/kill" -s 0 $p
eq "kill -s NOSUCH: process untouched" 0 $?
"$OURS/kill" $p
reap $p
eq "kill: default is TERM" 143 $?

for spec in "-s KILL:137" "-KILL:137" "-9:137" "-s SIGKILL:137" "-s HUP:129" "-HUP:129" "-s 15:143" "-15:143" "-s term:143"; do
	args=${spec%:*}
	want=${spec#*:}
	{
		sleep 30 < /dev/null > /dev/null 2>&1 &
		p=$!
		"$OURS/kill" $args $p
		wait $p
		eq "kill $args" "$want" $?
	} 2> /dev/null
done

sleep 0 &
gone=$!
wait $gone
sleep 30 < /dev/null > /dev/null 2>&1 &
a=$!
sleep 30 < /dev/null > /dev/null 2>&1 &
b=$!
"$OURS/kill" $a $gone $b 2> /dev/null
eq "kill with one unknown pid: status" 1 $?
reap $a
reap $b
"$OURS/kill" -s 0 $a 2> /dev/null
sa=$?
"$OURS/kill" -s 0 $b 2> /dev/null
eq "kill with one unknown pid: the others still signalled" "1 1" "$sa $?"
"$OURS/kill" $gone 2> /dev/null
eq "kill of a gone pid: status" 1 $?
"$OURS/kill" abc 2> /dev/null
eq "kill abc: status" 1 $?
"$OURS/kill" 2> /dev/null
eq "kill with no operands: usage status" 2 $?
eq "kill -l 143" TERM "$("$OURS/kill" -l 143)"
eq "kill -l 9" KILL "$("$OURS/kill" -l 9)"
"$OURS/kill" -l 0 > /dev/null 2>&1
eq "kill -l 0: not a signal" 1 $?

#------------------------------------------------- env nice time nohup
fresh
: > noexec
chmod 644 noexec
for u in env nice time nohup; do
	case $u in
	nice) a="-n 0" ;;
	time) a="-p" ;;
	*) a="" ;;
	esac
	out=$( (trap '' PIPE; "$OURS/$u" $a sh -c 'kill -PIPE $$; echo alive') 2> /dev/null)
	eq "$u: an inherited SIGPIPE ignore survives" alive "$out"
	"$OURS/$u" $a sh -c 'exit 42' > /dev/null 2>&1
	eq "$u: exit status of the utility" 42 $?
	"$OURS/$u" $a ./nosuchcmd > /dev/null 2>&1
	eq "$u: utility not found" 127 $?
	"$OURS/$u" $a ./noexec > /dev/null 2>&1
	eq "$u: utility not executable" 126 $?
	out=$("$OURS/$u" $a sh -c 'trap - PIPE; kill -s 0 $$ && echo ok' 2> /dev/null)
	eq "$u: runs the utility" ok "$out"
done

eq "env -i env: nothing" "" "$("$OURS/env" -i "$OURS/env")"
eq "env -i A=1 env" A=1 "$("$OURS/env" -i A=1 "$OURS/env")"
eq "env A=1 A=2: last wins" A=2 "$("$OURS/env" -i A=1 A=2 "$OURS/env")"
eq "env A=1=2: value may contain =" A=1=2 "$("$OURS/env" -i A=1=2 "$OURS/env")"
eq "env A= : empty value kept" A= "$("$OURS/env" -i A= "$OURS/env")"

"$OURS/time" -p true 2> t
eq "time -p: three lines" 3 "$(wc -l < t | tr -d ' ')"
eq "time -p: format" 3 "$(grep -Ec '^(real|user|sys) [0-9]+\.[0-9][0-9]$' t)"
"$OURS/time" -p sh -c 'kill -9 $$' > /dev/null 2>&1
eq "time: a signalled utility gives 128+n" 137 $?

out=$("$OURS/nice" -n -5 sh -c 'echo ran' 2> /dev/null)
eq "nice -n -5: runs the utility even if it cannot raise priority" ran "$out"
"$OURS/nice" -n 5 sh -c 'exit 3'
eq "nice: status" 3 $?

# nice: the values really change (ps shows them); this system stops at 19
nv='ps -o nice= -p $$'
base=$(ps -o nice= -p $$ | tr -d ' ')
# add A B: A + B, at most 19
add() {
	s=$(($1 + $2))
	[ "$s" -gt 19 ] && s=19
	echo "$s"
}
eq "nice: the default increment is 10" "$(add "$base" 10)" "$("$OURS/nice" sh -c "$nv" | tr -d ' ')"
eq "nice -n 0: unchanged" "$base" "$("$OURS/nice" -n 0 sh -c "$nv" | tr -d ' ')"
eq "nice -n 7" "$(add "$base" 7)" "$("$OURS/nice" -n 7 sh -c "$nv" | tr -d ' ')"
eq "nice nested: increments add up" "$(add "$base" 7)" "$("$OURS/nice" -n 3 "$OURS/nice" -n 4 sh -c "$nv" | tr -d ' ')"
eq "nice nested: the sum stops at the maximum" 19 "$("$OURS/nice" -n 15 "$OURS/nice" -n 15 sh -c "$nv" | tr -d ' ')"
eq "nice -n 100: clamped" 19 "$("$OURS/nice" -n 100 sh -c "$nv" | tr -d ' ')"
eq "nice: the parent keeps its own value" "$base" "$("$OURS/nice" -n 5 true; ps -o nice= -p $$ | tr -d ' ')"
eq "nice: arguments of the utility are not options" "-n|--|x" "$("$OURS/nice" -n 2 sh -c 'echo "$1|$2|$3"' sh -n -- x)"
eq "nice: standard input and output reach the utility" through "$(echo through | "$OURS/nice" cat)"
eq "nice: standard error reaches the caller" err "$("$OURS/nice" sh -c 'echo err >&2' 2>&1 > /dev/null)"

# time: what the figures mean
t=$("$OURS/time" -p sleep 1 2>&1)
eq "time -p sleep 1: real is a little over 1" ok "$(echo "$t" | awk '$1 == "real" { print ($2 >= 1 && $2 < 3) ? "ok" : "bad " $2 }')"
eq "time -p sleep 1: almost no CPU" ok "$(echo "$t" | awk '$1 == "user" || $1 == "sys" { s += $2 } END { print (s < 0.5) ? "ok" : "bad " s }')"
eq "time -p: real, user, sys in that order" "real user sys" "$(echo "$t" | awk '{ printf "%s ", $1 }' | sed 's/ $//')"
t=$("$OURS/time" -p sh -c 'awk "BEGIN { for (i = 0; i < 3000000; i++) x += i }"' 2>&1)
eq "time -p: CPU time of a grandchild is counted" ok "$(echo "$t" | awk '$1 == "user" || $1 == "sys" { s += $2 } END { print (s >= 0.05) ? "ok" : "bad " s }')"
out=$("$OURS/time" -p sh -c 'echo to-out; echo to-err >&2' 2> t)
eq "time: stdout carries only the utility's" to-out "$out"
eq "time: the utility's stderr comes first, then the figures" "to-err real user sys" "$(awk '{ printf "%s ", $1 }' t | sed 's/ $//')"
eq "time: standard input reaches the utility" through "$(echo through | "$OURS/time" -p cat 2> /dev/null)"
"$OURS/time" -p sh -c 'kill -9 $$' > /dev/null 2> t
eq "time: a signalled utility is still timed" 3 "$(grep -Ec '^(real|user|sys) ' t)"
if [ "$(LC_ALL=de_DE.UTF-8 locale decimal_point 2> /dev/null)" = , ]; then
	LC_ALL=de_DE.UTF-8 "$OURS/time" -p true 2> t
	eq "time -p uses the radix character of LC_NUMERIC" 3 "$(grep -Ec '^(real|user|sys) [0-9]+,[0-9][0-9]$' t)"
else
	echo "skip: no de_DE.UTF-8 locale"
fi
"$OURS/time" -p ./nosuchcmd > /dev/null 2> t
eq "time of a missing utility: status" 127 $?

#------------------------------------------------- nohup on a terminal
# script(1) gives the command a pseudo-terminal for stdin, stdout and stderr.
export OURS
if [ -x /usr/bin/script ] && /usr/bin/script -q /dev/null true < /dev/null > /dev/null 2>&1; then
	# ttyrun CMD: run the shell command CMD on a pseudo-terminal, screen text in ./ty
	ttyrun() {
		/usr/bin/script -q /dev/null sh -c "$1" < /dev/null > ty 2>&1
	}
	W=$(pwd -P)/tw
	export W
	rm -rf "$W"
	mkdir "$W" "$W/ro" "$W/home"
	cd "$W" || exit 1

	ttyrun '"$OURS/nohup" sh -c "echo out; echo err >&2"'
	eq "nohup on a terminal: output goes to nohup.out" "out err" "$(tr '\n' ' ' < nohup.out | sed 's/ $//')"
	eq "nohup on a terminal: created with mode 0600" -rw------- "$(mode nohup.out)"
	eq "nohup on a terminal: the screen shows the message, not the output" "1 0" \
		"$(grep -c 'nohup.out' ty) $(grep -Ec '^(out|err)' ty)"

	rm -f nohup.out
	printf 'old\n' > nohup.out
	chmod 644 nohup.out
	ttyrun '"$OURS/nohup" echo new'
	eq "nohup appends to an existing nohup.out" "old new" "$(tr '\n' ' ' < nohup.out | sed 's/ $//')"
	eq "nohup leaves the mode of an existing nohup.out" -rw-r--r-- "$(mode nohup.out)"
	rm -f nohup.out

	chmod 555 ro
	ttyrun 'cd "$W/ro" && HOME="$W/home" "$OURS/nohup" echo viahome'
	eq "nohup falls back to HOME/nohup.out" "viahome" "$(cat home/nohup.out)"
	eq "nohup in HOME: mode 0600" -rw------- "$(mode home/nohup.out)"
	eq "nohup in HOME: the message names that file" 1 "$(grep -c "$W/home/nohup.out" ty)"
	eq "nohup in HOME: nothing in the directory that failed" "" "$(ls ro)"
	rm -f home/nohup.out

	rm -f marker
	ttyrun 'cd "$W/ro" && HOME="$W/ro" "$OURS/nohup" touch "$W/marker"; echo $? > "$W/st"'
	eq "nohup with no place for nohup.out: status" 127 "$(cat st)"
	eq "nohup with no place for nohup.out: the utility is not run" "" "$([ -e marker ] && echo ran)"
	chmod 755 ro

	ttyrun '"$OURS/nohup" sh -c "echo o; echo e >&2" > f'
	eq "nohup: stderr on a terminal follows a redirected stdout" "o e" "$(tr '\n' ' ' < f | sed 's/ $//')"
	eq "nohup: nothing created when stdout is not a terminal" "" "$([ -e nohup.out ] && echo made)"

	ttyrun '"$OURS/nohup" sh -c "echo o; echo e >&2" 2> e2'
	eq "nohup: stdout on a terminal, stderr redirected: output in nohup.out" o "$(cat nohup.out)"
	eq "nohup: stdout on a terminal, stderr redirected: the message goes to stderr" "1" "$(grep -c nohup.out e2)"
	rm -f nohup.out

	ttyrun '"$OURS/nohup" sh -c "echo e >&2" >&-'
	eq "nohup: stderr on a terminal, stdout closed: nohup.out" e "$(cat nohup.out)"
	rm -f nohup.out

	ttyrun '"$OURS/nohup" echo hi > f 2> g'
	eq "nohup: no terminal for stdout or stderr: output untouched" "hi|0" "$(cat f)|$(wc -c < g | tr -d ' ')"
	eq "nohup: no terminal for stdout or stderr: no nohup.out" "" "$([ -e nohup.out ] && echo made)"

	ttyrun '"$OURS/nohup" sh -c "exit 5"; echo $? > "$W/st"'
	eq "nohup on a terminal: exit status of the utility" 5 "$(cat st)"
	ttyrun '"$OURS/nohup" ./nosuchcmd; echo $? > "$W/st"'
	eq "nohup on a terminal: utility not found" 127 "$(cat st)"
	: > noexec
	chmod 644 noexec
	ttyrun '"$OURS/nohup" ./noexec; echo $? > "$W/st"'
	eq "nohup on a terminal: utility not executable" 126 "$(cat st)"
	rm -f nohup.out

	ttyrun '"$OURS/nohup" sh -c "kill -HUP \$\$; echo survived"'
	eq "nohup on a terminal: SIGHUP is ignored by the utility" survived "$(cat nohup.out)"
	cd "$tmp" || exit 1
	rm -rf "$W"
	fresh
else
	echo "skip: no script(1)"
fi

#----------------------------------------------------- sleep tty logname
fresh
"$OURS/sleep" 0
eq "sleep 0" 0 $?
"$OURS/sleep" 1.5 2> /dev/null
eq "sleep 1.5: refused" 1 $?
"$OURS/sleep" 99999999999999999999999 < /dev/null > /dev/null 2>&1
eq "sleep: a time that overflows 64 bits is refused" 1 $?
"$OURS/sleep" 18446744073709551615 < /dev/null > /dev/null 2>&1 &
p=$!
sleep 1
"$OURS/kill" -s 0 $p
eq "sleep with the largest time: still asleep after a second" 0 $?
"$OURS/kill" $p
reap $p
t0=$(date +%s)
"$OURS/sleep" 1
t1=$(date +%s)
eq "sleep 1: takes at least a second, not much more" ok "$([ $((t1 - t0)) -ge 1 ] && [ $((t1 - t0)) -le 2 ] && echo ok)"
"$OURS/sleep" 100 &
p=$!
sleep 1
kill -TERM $p
reap $p
eq "sleep: SIGTERM ends it with the default action" 143 $?
{
	"$OURS/sleep" 100 &
	p=$!
	sleep 1
	kill -ALRM $p
	sleep 1
	if kill -0 $p; then
		kill $p
		reap $p
		r=ignored
	else
		reap $p
		r=$?
	fi
} 2> /dev/null
case $r in
ignored | 0 | 142) r=allowed ;;
esac
eq "sleep: SIGALRM is ignored, exits 0 or takes the default action" allowed "$r"
"$OURS/sleep" 0 < /dev/null > out 2> err
eq "sleep 0: no output at all" "0 0" "$(wc -c < out | tr -d ' ') $(wc -c < err | tr -d ' ')"
"$OURS/sleep" 0 <&- >&- 2>&-
eq "sleep 0 with every standard descriptor closed" 0 $?
"$OURS/sleep" 1 1 2> /dev/null
eq "sleep 1 1: one operand only" 2 $?
"$OURS/tty" < /dev/null > out
eq "tty on /dev/null: status" 1 $?
eq "tty on /dev/null: output" "not a tty" "$(cat out)"
"$OURS/tty" -s < /dev/null 2> /dev/null
eq "tty -s: not an option of the 2018 edition" 2 $?

#---------------------------------------------------------------------- uname
# the system's own figures, from sysctl, are the reference
fresh
s=$(sysctl -n kern.ostype)
n=$(sysctl -n kern.hostname)
r=$(sysctl -n kern.osrelease)
v=$(sysctl -n kern.version)
m=$(sysctl -n hw.machine)
eq "uname: no option means -s" "$s" "$("$OURS/uname")"
eq "uname -s" "$s" "$("$OURS/uname" -s)"
eq "uname -n" "$n" "$("$OURS/uname" -n)"
eq "uname -r" "$r" "$("$OURS/uname" -r)"
eq "uname -v" "$v" "$("$OURS/uname" -v)"
eq "uname -m" "$m" "$("$OURS/uname" -m)"
eq "uname -a: all five, in the order of the format" "$s $n $r $v $m" "$("$OURS/uname" -a)"
eq "uname -a: one line" 1 "$("$OURS/uname" -a | wc -l | tr -d ' ')"
eq "uname -mnrsv: the order does not depend on the options" "$s $n $r $v $m" "$("$OURS/uname" -mnrsv)"
eq "uname -vm -s: options given in two words" "$s $v $m" "$("$OURS/uname" -vm -s)"
eq "uname -rn: no trailing blank" "$n $r" "$("$OURS/uname" -rn)"
eq "uname -a -s: -a stays the whole set" "$s $n $r $v $m" "$("$OURS/uname" -a -s)"
eq "uname -s ends in one newline" "$(printf '%s\n_' "$s")" "$("$OURS/uname" -s; echo _)"
"$OURS/uname" -x > out 2> err
eq "uname -x: usage error" "2 0" "$? $(wc -c < out | tr -d ' ')"
eq "uname -x: a diagnostic" 1 "$(wc -l < err | tr -d ' ')"
"$OURS/uname" extra > out 2> /dev/null
eq "uname with an operand: usage error, no output" "2 0" "$? $(wc -c < out | tr -d ' ')"
"$OURS/uname" -a >&- 2> err
eq "uname with stdout closed: status" 1 $?
eq "uname with stdout closed: a diagnostic" 1 "$(wc -l < err | tr -d ' ')"

#------------------------------------------------------- wc head tail tsort
fresh
seq 1 30 > n30
seq 1 5 > n5
awk 'BEGIN { for (i = 1; i <= 1500000; i++) print i }' > big
printf 'a b c\n' > abc

eq "wc from a pipe" "1 3 6" "$(cat abc | "$OURS/wc")"
eq "wc -l of a long pipe" 100000 "$(seq 1 100000 | "$OURS/wc" -l)"
eq "wc of two files has a total" "1 3 6 abc
1 3 6 abc
2 6 12 total" "$("$OURS/wc" abc abc)"
eq "wc -c of a big file" "$(wc -c < big | tr -d ' ')" "$("$OURS/wc" -c < big)"
eq "wc of a big file through a pipe" "1500000 1500000 $(wc -c < big | tr -d ' ')" "$(cat big | "$OURS/wc")"
"$OURS/wc" abc >&- 2> err
eq "wc with stdout closed: status" 1 $?
eq "wc with stdout closed: diagnostic" 1 "$(wc -l < err | tr -d ' ')"
"$OURS/wc" <&- 2> /dev/null
eq "wc with stdin closed: status" 1 $?
"$OURS/wc" . 2> /dev/null
eq "wc of a directory: status" 1 $?
"$OURS/wc" nosuch abc > out 2> /dev/null
eq "wc with a missing file: status" 1 $?
eq "wc with a missing file: the rest is counted, with a total" "1 3 6 abc
1 3 6 total" "$(cat out)"
if [ "$(LC_ALL=en_US.UTF-8 locale charmap 2> /dev/null)" = UTF-8 ]; then
	E=$(printf '\303\251')
	# 100000 lines of "e-acute newline": 64 KB blocks cut some of the characters in two
	eq "wc -mw counts characters whole over a pipe" "100000 200000" \
		"$(yes "$E" | head -n 100000 | LC_ALL=en_US.UTF-8 "$OURS/wc" -wm)"
	eq "wc -c over the same pipe counts bytes" 300000 \
		"$(yes "$E" | head -n 100000 | LC_ALL=en_US.UTF-8 "$OURS/wc" -c)"
	eq "wc -m in the C locale counts bytes" 300000 \
		"$(yes "$E" | head -n 100000 | LC_ALL=C "$OURS/wc" -m)"
else
	echo "skip: no en_US.UTF-8 locale"
fi

eq "head of an endless pipe stops" "1 2 3" "$(seq 1 2000000000 | "$OURS/head" -n 3 | tr '\n' ' ' | sed 's/ $//')"
eq "head of a big file" "1 2 3 4 5" "$("$OURS/head" -n 5 big | tr '\n' ' ' | sed 's/ $//')"
eq "head -n 1500000 of a big file is all of it" 0 "$("$OURS/head" -n 1500000 big | cmp - big > /dev/null; echo $?)"
eq "head leaves the shared offset after the last line" "1 2 3 4" \
	"$({ "$OURS/head" -n 2; "$OURS/head" -n 2; } < n30 | tr '\n' ' ' | sed 's/ $//')"
eq "head -n 2 and then cat share the offset" "1 2 3 4 5" \
	"$({ "$OURS/head" -n 2; "$OURS/cat"; } < n5 | tr '\n' ' ' | sed 's/ $//')"
eq "head - reads standard input" "1 2" "$("$OURS/head" -n 2 - < n5 | tr '\n' ' ' | sed 's/ $//')"
{ "$OURS/head" -n 1000000 big; echo $? > st; } | head -c 10 > /dev/null
eq "head into a closed pipe dies of SIGPIPE" 141 "$(cat st)"
"$OURS/head" n5 >&- 2> err
eq "head with stdout closed: status" 1 $?
eq "head with stdout closed: diagnostic" 1 "$(wc -l < err | tr -d ' ')"
"$OURS/head" <&- 2> /dev/null
eq "head with stdin closed: status" 1 $?
"$OURS/head" nosuch n5 > out 2> /dev/null
eq "head with a missing file: status" 1 $?
eq "head with a missing file: no header for it, none for the first" "==> n5 <==
1
2
3
4
5" "$(cat out)"
eq "head with two files" "==> n5 <==
1

==> n5 <==
1" "$("$OURS/head" -n 1 n5 n5)"

eq "tail of a pipe: last lines" "99998 99999 100000" "$(seq 1 100000 | "$OURS/tail" -n 3 | tr '\n' ' ' | sed 's/ $//')"
eq "tail of a pipe: last bytes" "100000" "$(seq 1 100000 | "$OURS/tail" -c 7)"
eq "tail of a pipe: from a line" "99999 100000" "$(seq 1 100000 | "$OURS/tail" -n +99999 | tr '\n' ' ' | sed 's/ $//')"
eq "tail of a pipe: all of a big stream" 0 "$(cat big | "$OURS/tail" -n 1500000 | cmp - big > /dev/null; echo $?)"
eq "tail of a pipe: more than there is" 0 "$(cat big | "$OURS/tail" -n 99999999 | cmp - big > /dev/null; echo $?)"
eq "tail of a pipe: more bytes than there are" 0 "$(cat big | "$OURS/tail" -c 99999999 | cmp - big > /dev/null; echo $?)"
eq "tail of a big file: last lines" "1499998 1499999 1500000" "$("$OURS/tail" -n 3 big | tr '\n' ' ' | sed 's/ $//')"
eq "tail of a big file: from the first line is all of it" 0 "$("$OURS/tail" -n +1 big | cmp - big > /dev/null; echo $?)"
eq "tail of a big file: from the first byte is all of it" 0 "$("$OURS/tail" -c +1 big | cmp - big > /dev/null; echo $?)"
eq "tail of a big file: last bytes" "1500000" "$("$OURS/tail" -c 8 big | tr -d '\n' | sed 's/^0*//')"
eq "tail of a big file: same through stdin" "1499999 1500000" "$("$OURS/tail" -n 2 < big | tr '\n' ' ' | sed 's/ $//')"
eq "tail -n +N skips from the start" "28 29 30" "$("$OURS/tail" -n +28 n30 | tr '\n' ' ' | sed 's/ $//')"
eq "tail -n 0 prints nothing" 0 "$("$OURS/tail" -n 0 n30 | wc -c | tr -d ' ')"
eq "tail counts from the offset of a shared stdin" "2 3 4 5" \
	"$({ read x; "$OURS/tail" -n +1; } < n5 | tr '\n' ' ' | sed 's/ $//')"
eq "tail -n 10 on a shared stdin starts at the offset" "2 3 4 5" \
	"$({ read x; "$OURS/tail"; } < n5 | tr '\n' ' ' | sed 's/ $//')"
"$OURS/tail" n5 >&- 2> err
eq "tail with stdout closed: status" 1 $?
eq "tail with stdout closed: diagnostic" 1 "$(wc -l < err | tr -d ' ')"
"$OURS/tail" <&- 2> /dev/null
eq "tail with stdin closed: status" 1 $?
"$OURS/tail" . 2> /dev/null
eq "tail of a directory: status" 1 $?
"$OURS/tail" nosuch 2> /dev/null
eq "tail of a missing file: status" 1 $?
"$OURS/tail" n5 n5 2> /dev/null
eq "tail with two files is a usage error" 2 $?
{ "$OURS/tail" -n 1000000 big; echo $? > st; } | head -c 10 > /dev/null
eq "tail into a closed pipe dies of SIGPIPE" 141 "$(cat st)"

# tail -f: appended data arrives, the process keeps running; poll for up to 5 s
waitfor() {
	i=0
	while [ $i -lt 50 ] && [ "$(cat "$1" 2> /dev/null)" != "$2" ]; do
		sleep 0.1
		i=$((i + 1))
	done
}
printf 'old\n' > grow
"$OURS/tail" -f grow > fout &
p=$!
waitfor fout old
echo new >> grow
waitfor fout "old
new"
eq "tail -f: old and appended lines" "old
new" "$(cat fout)"
kill -0 $p 2> /dev/null
eq "tail -f: still running" 0 $?
kill $p
reap $p
printf 'old\n' > grow
"$OURS/tail" -f -n 0 grow > fout &
p=$!
sleep 0.5
echo new >> grow
waitfor fout new
eq "tail -f -n 0: only what is appended" new "$(cat fout)"
kill $p
reap $p
eq "tail -f on a pipe is ignored" b "$(printf 'a\nb\n' | "$OURS/tail" -f -n 1)"
rm -f ff
mkfifo ff
"$OURS/tail" -f ff > fout &
p=$!
echo hello > ff
waitfor fout hello
eq "tail -f on a named FIFO: data arrives" hello "$(cat fout)"
sleep 0.3
kill -0 $p 2> /dev/null
eq "tail -f on a named FIFO: keeps running after end of file" 0 $?
kill $p
reap $p

eq "tsort: the example of the spec" "a b c d e f g h" \
	"$(printf 'a b\nc c\nd e\ng g\nf g\ne f\nh h\n' | "$OURS/tsort" | tr '\n' ' ' | sed 's/ $//')"
awk 'BEGIN { srand(7); for (i = 0; i < 40000; i++) { a = int(rand() * 5000); b = int(rand() * 5000); if (a > b) { t = a; a = b; b = t }; print "n" a, "n" b } }' > dag
"$OURS/tsort" dag > order
eq "tsort of a big random order: status" 0 $?
eq "tsort of a big random order: every pair in order" 0 \
	"$(awk 'NR == FNR { pos[$1] = FNR; next } { if (pos[$1] == "" || pos[$2] == "" || ($1 != $2 && pos[$1] >= pos[$2])) bad++ } END { print bad + 0 }' order dag)"
eq "tsort of a big random order: each item once" "$(tr -s ' \n' '\n\n' < dag | sort -u | wc -l | tr -d ' ')" "$(sort order | uniq | wc -l | tr -d ' ')"
eq "tsort of a big random order: no item twice" "$(wc -l < order | tr -d ' ')" "$(sort -u order | wc -l | tr -d ' ')"
printf 'a b\nb a\nc d\n' | "$OURS/tsort" > out 2> err
eq "tsort cycle: status" 1 $?
eq "tsort cycle: every item still printed" "a b c d" "$(sort out | tr '\n' ' ' | sed 's/ $//')"
eq "tsort cycle: diagnostic" 1 "$(grep -c 'cycle in data' err)"
printf 'a b c\n' | "$OURS/tsort" > out 2> err
eq "tsort odd data: status" 1 $?
eq "tsort odd data: nothing printed" 0 "$(wc -c < out | tr -d ' ')"
eq "tsort odd data: diagnostic" 1 "$(wc -l < err | tr -d ' ')"
printf 'a b\n' | "$OURS/tsort" >&- 2> err
eq "tsort with stdout closed: status" 1 $?
"$OURS/tsort" <&- 2> /dev/null
eq "tsort with stdin closed: status" 1 $?
"$OURS/tsort" nosuch 2> /dev/null
eq "tsort of a missing file: status" 1 $?
"$OURS/tsort" n5 n5 2> /dev/null
eq "tsort with two files is a usage error" 2 $?

# "-" is standard input here; the pieces of a slow pipe are joined into items
eq "tsort -: standard input" "a b" "$(printf 'a b\n' | "$OURS/tsort" - | tr '\n' ' ' | sed 's/ $//')"
eq "tsort: a pair arriving in pieces over a slow pipe" "a b c" \
	"$({ printf 'a '; sleep 1; printf 'b\nb c\n'; } | "$OURS/tsort" | tr '\n' ' ' | sed 's/ $//')"
eq "tsort: an item cut in two by a slow pipe" "ab c" \
	"$({ printf 'a'; sleep 1; printf 'b c\n'; } | "$OURS/tsort" | tr '\n' ' ' | sed 's/ $//')"

# the report of a cycle names the items on it and nothing else
printf 'a b\nb c\nc a\nc d\n' | "$OURS/tsort" > out 2> err
eq "tsort cycle of three: status" 1 $?
eq "tsort cycle of three: every item once" "a b c d" "$(sort out | tr '\n' ' ' | sed 's/ $//')"
eq "tsort cycle of three: the report names a b c" "a b c" "$(grep -v 'cycle in data' err | sed 's/^tsort: //' | sort | tr '\n' ' ' | sed 's/ $//')"
printf 'a b\nb a\nc d\nd c\n' | "$OURS/tsort" > out 2> err
eq "tsort two separate cycles: both reported" 2 "$(grep -c 'cycle in data' err)"
eq "tsort two separate cycles: every item once" "a b c d" "$(sort out | tr '\n' ' ' | sed 's/ $//')"
printf 'a a\n' | "$OURS/tsort" > out 2> err
eq "tsort a a: an item before itself is no cycle" "0 a 0" "$? $(cat out) $(wc -c < err | tr -d ' ')"

# very long chains are sorted without recursion; a small stack would show it
awk 'BEGIN { for (i = 1; i < 1000000; i++) printf "%d %d\n", i, i + 1 }' > chain
awk 'BEGIN { for (i = 999999; i >= 1; i--) printf "%d %d\n", i, i + 1 }' > rchain
awk 'BEGIN { for (i = 1; i <= 1000000; i++) printf "%d\n", i }' > seq1m
(
	ulimit -s 1024
	"$OURS/tsort" chain > out1
	echo $? > st1
	"$OURS/tsort" rchain > out2
	echo $? > st2
)
eq "tsort of a chain of a million items, small stack: status" "0 0" "$(cat st1) $(cat st2)"
eq "tsort of a chain of a million items: in order" 0 "$(cmp out1 seq1m > /dev/null 2>&1; echo $?)"
eq "tsort of the same chain written backwards: in order" 0 "$(cmp out2 seq1m > /dev/null 2>&1; echo $?)"
{ "$OURS/tsort" chain; echo $? > st; } | head -c 10 > /dev/null
eq "tsort into a closed pipe dies of SIGPIPE" 141 "$(cat st)"
eq "tsort: a million copies of one pair" "a b" "$(yes 'a b' | head -n 1000000 | "$OURS/tsort" | tr '\n' ' ' | sed 's/ $//')"
awk 'BEGIN { s = sprintf("%1000000s", ""); gsub(/ /, "y", s); print s, "z" }' > longitem
"$OURS/tsort" longitem > out
eq "tsort: an item of a million bytes" "1000001 2" "$(head -n 1 out | wc -c | tr -d ' ') $(wc -l < out | tr -d ' ')"

#---------------------------------------------------------------------- uniq
fresh
printf 'a\na\nb\n' > f
"$OURS/uniq" f f 2> err
eq "uniq f f: status" 1 $?
eq "uniq f f: input kept" "a a b" "$(tr '\n' ' ' < f | sed 's/ $//')"
eq "uniq f f: message" "uniq: f: input file is output file" "$(cat err)"
"$OURS/uniq" f g
eq "uniq f g: status" 0 $?
eq "uniq f g: output file" "a b" "$(tr '\n' ' ' < g | sed 's/ $//')"
printf 'old old old old old old\n' > g
"$OURS/uniq" f g
eq "uniq into a longer file truncates it" "a b" "$(tr '\n' ' ' < g | sed 's/ $//')"
printf 'keep\n' > g
"$OURS/uniq" nosuch g 2> /dev/null
eq "uniq of a missing input leaves the output file alone" "keep" "$(cat g)"
"$OURS/uniq" f - | tr '\n' ' ' > out
eq "uniq with - as output is standard output" "a b " "$(cat out)"
printf 'a\na\n' | "$OURS/uniq" - | tr '\n' ' ' > out
eq "uniq - reads standard input" "a " "$(cat out)"
"$OURS/uniq" -d -u f 2> /dev/null
eq "uniq -d -u is a usage error" 2 $?
"$OURS/uniq" f g h 2> /dev/null
eq "uniq with three operands is a usage error" 2 $?
"$OURS/uniq" -f x f 2> /dev/null
eq "uniq -f x is a usage error" 2 $?
"$OURS/uniq" f >&- 2> /dev/null
eq "uniq with stdout closed: status" 1 $?
"$OURS/uniq" <&- 2> /dev/null
eq "uniq with stdin closed: status" 1 $?
awk 'BEGIN { for (i = 0; i < 200000; i++) print i % 7 }' | "$OURS/uniq" | head -n 2 > out
eq "uniq into a closed pipe ends quietly" "0 1" "$(tr '\n' ' ' < out | sed 's/ $//')"

#---------------------------------------------------------------------- test
# the exit status tells a false result (1) from an error (2); [ is the same program
fresh
T="$OURS/test"
B="$OURS/["
"$T" a = a
eq "test true: status" 0 $?
"$T" a = b
eq "test false: status" 1 $?
"$T"
eq "test with no arguments: false" 1 $?
"$T" a = b c 2> err
eq "test a = b c: error status and a diagnostic" "2 1" "$? $(wc -l < err | tr -d ' ')"
"$T" x -eq 1 2> /dev/null
eq "test x -eq 1: error status" 2 $?
"$T" -t x 2> /dev/null
eq "test -t x: error status" 2 $?
"$B" a = a ']'
eq "[ a = a ]: status" 0 $?
"$B" a = b ']'
eq "[ a = b ]: status" 1 $?
"$B" a = a 2> err
eq "[ without ]: error status and a diagnostic" "2 1" "$? $(wc -l < err | tr -d ' ')"
"$B" ']'
eq "[ ]: false" 1 $?
"$B" 2> /dev/null
eq "[ alone: error status" 2 $?
"$T" a = a ']'  2> /dev/null
eq "test does not take ]: error status" 2 $?
"$T" -t 0 < /dev/null
eq "test -t 0 of /dev/null: false" 1 $?
"$T" -t 9 9< /dev/null
eq "test -t on a descriptor that is not a terminal: false" 1 $?
"$T" -e /dev/null >&-
eq "test with stdout closed still works" 0 $?
"$T" -d . <&-
eq "test with stdin closed still works" 0 $?
touch a; chmod 000 a
"$T" -r a
st=$?
want=1
[ "$(id -u)" = 0 ] && want=0
eq "test -r of a file without permission" $want $st
rm -f a

#---------------------------------------------------------------------- sort
fresh
printf 'b\na\nc\n' > f
printf 'a\nb\nc\n' > ok
"$OURS/sort" f > out
eq "sort: status and output" "0 a b c" "$? $(tr '\n' ' ' < out | sed 's/ $//')"
"$OURS/sort" -o f f
eq "sort -o f f: status" 0 $?
eq "sort -o f f: the input file is sorted in place" "a b c" "$(tr '\n' ' ' < f | sed 's/ $//')"
printf 'b\na\n' > f
"$OURS/sort" -o g f f > /dev/null
eq "sort -o: output file" "a a b b" "$(tr '\n' ' ' < g | sed 's/ $//')"
"$OURS/sort" -c ok 2> err
eq "sort -c of sorted input: status, silence" "0 0" "$? $(wc -c < err | tr -d ' ')"
printf 'a\nc\nb\n' > un
"$OURS/sort" -c un > out 2> err
eq "sort -c of unsorted input: status" 1 $?
eq "sort -c: nothing on standard output" 0 "$(wc -c < out | tr -d ' ')"
eq "sort -c: the line is named" "sort: un:3: disorder: b" "$(cat err)"
"$OURS/sort" -C un 2> err
eq "sort -C: status and silence" "1 0" "$? $(wc -c < err | tr -d ' ')"
printf 'a\na\n' | "$OURS/sort" -cu 2> err
eq "sort -cu of a duplicate: status" 1 $?
eq "sort -cu: message names standard input" "sort: -:2: disorder: a" "$(cat err)"
"$OURS/sort" nosuch f > out 2> err
eq "sort of a missing file: status, no output" "2 0" "$? $(wc -c < out | tr -d ' ')"
"$OURS/sort" -o nosuchdir/x f 2> /dev/null
eq "sort -o into a missing directory: status" 2 $?
"$OURS/sort" -k 0 f 2> /dev/null
eq "sort -k 0 is a usage error" 2 $?
"$OURS/sort" -c -m ok 2> /dev/null
eq "sort -c -m is a usage error" 2 $?
"$OURS/sort" -t ab f 2> /dev/null
eq "sort -t ab is a usage error" 2 $?
"$OURS/sort" f >&- 2> /dev/null
eq "sort with stdout closed: status" 2 $?
"$OURS/sort" <&- 2> /dev/null
eq "sort with stdin closed: status" 2 $?
printf 'b\na' | "$OURS/sort" > out
eq "sort adds the missing newline" "a b" "$(tr '\n' ' ' < out | sed 's/ $//')"
printf '3\n1\n2\n' | "$OURS/sort" -n - ok > out
eq "sort - reads standard input among files" "a b c 1 2 3" "$(tr '\n' ' ' < out | sed 's/ $//')"
awk 'BEGIN { for (i = 200000; i > 0; i--) print i }' | "$OURS/sort" -n | head -n 3 > out
eq "sort into a closed pipe ends quietly" "1 2 3" "$(tr '\n' ' ' < out | sed 's/ $//')"
awk 'BEGIN { srand(5); for (i = 0; i < 300000; i++) print int(rand() * 100000) }' > big
"$OURS/sort" -n big > out
eq "sort -n of a big file: status" 0 $?
eq "sort -n of a big file: same line count" "$(wc -l < big | tr -d ' ')" "$(wc -l < out | tr -d ' ')"
"$OURS/sort" -c -n out
eq "sort -c -n agrees with sort -n on a big file" 0 $?
"$OURS/sort" -u -n big > out
eq "sort -u -n of a big file: distinct values only" "$(sort -n big | uniq | wc -l | tr -d ' ')" "$(wc -l < out | tr -d ' ')"

#---------------------------------------------------------------------- expr
# the exit status tells a false result (1) from an invalid expression (2)
fresh
"$OURS/expr" 1 + 1 > out
eq "expr true: status and output" "0 2" "$? $(cat out)"
"$OURS/expr" 1 - 1 > out
eq "expr zero: status and output" "1 0" "$? $(cat out)"
"$OURS/expr" a : b > out
eq "expr no match: status and output" "1 0" "$? $(cat out)"
"$OURS/expr" a : '\(b\)' > out
eq "expr empty subexpression: status and a null line" "1 1" "$? $(wc -l < out | tr -d ' ')"
"$OURS/expr" '' > out
eq "expr of null: status and a null line" "1 1" "$? $(wc -l < out | tr -d ' ')"
"$OURS/expr" 1 + 2> /dev/null > out
eq "expr 1 + : syntax error status" 2 $?
"$OURS/expr" 2> err > out
eq "expr without operands: status" 2 $?
"$OURS/expr" 1 / 0 2> err > out
eq "expr 1 / 0: status, no output, a diagnostic" "2 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
"$OURS/expr" 9223372036854775807 + 1 2> /dev/null > out
eq "expr overflow: status above 2" 3 $?
"$OURS/expr" a : '\(' 2> /dev/null > out
eq "expr invalid pattern: status 2" 2 $?
"$OURS/expr" -- -3 + 1 > out
eq "expr -- -3 + 1" "0 -2" "$? $(cat out)"
"$OURS/expr" -3 + 1 > out
eq "expr -3 + 1 (a negative integer without --)" "0 -2" "$? $(cat out)"
"$OURS/expr" 1 + 1 >&- 2> /dev/null
eq "expr with stdout closed: status" 1 $?
"$OURS/expr" 1 \| 1 / 0 > out
eq "expr 1 | 1 / 0 leaves the right side alone" "0 1" "$? $(cat out)"

#-------------------------------------------------- bytes, not UTF-8
fresh
B=$(printf '\377\376')
eq "basename keeps raw bytes" fffe0a "$("$OURS/basename" "x/$B.c" .c | hex)"
eq "dirname keeps raw bytes" fffe0a "$("$OURS/dirname" "$B/x" | hex)"
eq "echo keeps raw bytes" fffe0a "$("$OURS/echo" "$B" | hex)"
eq "env keeps raw bytes in values" fffe "$("$OURS/env" "A=$B" sh -c 'printf %s "$A"' | hex)"
eq "pathchk -p shows a stray byte as \\xNN" 1 "$("$OURS/pathchk" -p "a/x$B" 2>&1 | grep -cF 'x\xff\xfe')"
if mkdir "z$B" 2> /dev/null; then
	rmdir "z$B"
	"$OURS/mkdir" "z$B"
	eq "mkdir with raw bytes in the name" 0 $?
	"$OURS/rmdir" "z$B"
	eq "rmdir with raw bytes in the name" "0 gone" "$? $([ -e "z$B" ] || echo gone)"
	echo x | "$OURS/tee" "t$B" > /dev/null
	eq "tee with raw bytes in the name" x "$(cat "t$B")"
	eq "cat with raw bytes in the name" x "$("$OURS/cat" "t$B")"
else
	echo "skip: this file system refuses non-UTF-8 names"
fi

#---------------------------------------------------------- expand unexpand
fresh
BIGTABS=$(printf 'a\tb')
yes "$BIGTABS" | head -c 5000000 > bigtabs
yes '        x' | head -n 100000 > bigspaces

for u in expand unexpand; do
	printf 'a\tb\n' | "$OURS/$u" >&- 2> err
	eq "$u with stdout closed: status" 1 $?
	eq "$u with stdout closed: diagnostic" 1 "$(wc -l < err | tr -d ' ')"
	"$OURS/$u" <&- > /dev/null 2> err
	eq "$u with stdin closed: status" 1 $?
	{ "$OURS/$u" bigtabs; echo $? > st; } | head -c 10 > /dev/null
	eq "$u into a closed pipe dies of SIGPIPE" 141 "$(cat st)"
	(trap '' PIPE; { "$OURS/$u" bigtabs 2> err; echo $? > st; } | head -c 10 > /dev/null)
	eq "$u into a closed pipe with SIGPIPE ignored: status" 1 "$(cat st)"
	eq "$u into a closed pipe with SIGPIPE ignored: diagnostic" 1 "$(wc -l < err | tr -d ' ')"
	"$OURS/$u" . > /dev/null 2> err
	eq "$u of a directory: status" 1 $?
	printf 'x\n' > f
	"$OURS/$u" nosuch f > /dev/null 2>&1
	eq "$u nosuch f: status" 1 $?
	eq "$u - reads standard input" x "$(printf x | "$OURS/$u" - )"
	eq "$u keeps NUL and high bytes" 0061ff0a "$(printf '\000a\377\n' | "$OURS/$u" | hex)"
	"$OURS/$u" -t 4 -- f > /dev/null
	eq "$u -t 4 -- f" 0 $?
	"$OURS/$u" -t 0 f > /dev/null 2> err
	eq "$u -t 0: status, no output, a diagnostic" "1 1" "$? $(wc -l < err | tr -d ' ')"
	"$OURS/$u" -t 3,3 f > o 2> /dev/null
	eq "$u -t 3,3: status and no output" "1 0" "$? $(wc -c < o | tr -d ' ')"
	"$OURS/$u" -x f > o 2> /dev/null
	eq "$u -x: usage error, status 2" "2 0" "$? $(wc -c < o | tr -d ' ')"
done

# expand stops at the first operand it cannot access, unexpand goes on (PCTS 824)
printf 'x\n' > f
eq "expand nosuch f: terminates, nothing written" "" "$("$OURS/expand" nosuch f 2> /dev/null)"
eq "unexpand nosuch f: the later file is still read" x "$("$OURS/unexpand" nosuch f 2> /dev/null)"
eq "expand f nosuch f: what came before stays, the rest is not read" x "$("$OURS/expand" f nosuch f 2> /dev/null)"
eq "unexpand f nosuch f: every readable file is copied" "x x" "$("$OURS/unexpand" f nosuch f 2> /dev/null | tr '\n' ' ' | sed 's/ $//')"
"$OURS/expand" f nosuch f > /dev/null 2>&1
eq "expand f nosuch f: status" 1 $?
"$OURS/expand" . f > o 2> /dev/null
eq "expand . f: a directory ends it too" "1 0" "$? $(wc -c < o | tr -d ' ')"

# output comes as the input arrives, not at its end
start=$(date +%s)
{ printf 'a\tb\n'; sleep 3; } | "$OURS/expand" | { read -r line; date +%s > t1; }
eq "expand answers before its input ends" 1 "$([ $(($(cat t1) - start)) -lt 3 ] && echo 1 || echo 0)"
start=$(date +%s)
{ printf '        x\n'; sleep 3; } | "$OURS/unexpand" | { read -r line; date +%s > t1; }
eq "unexpand answers before its input ends" 1 "$([ $(($(cat t1) - start)) -lt 3 ] && echo 1 || echo 0)"

# expand
eq "expand: default stops" "6120202020202020620a" "$(printf 'a\tb\n' | "$OURS/expand" | hex)"
eq "expand -t 3,4: a tab past the last stop is one space" "612020622063" "$(printf 'a\tb\tc' | "$OURS/expand" -t 3,4 | hex)"
eq "expand: the column count runs on between files" "61626364202020207a" "$(printf 'abcd' > f1; printf '\tz' > f2; "$OURS/expand" f1 f2 | hex)"
for n in 65535 65536 65537 131071 131072 131073; do
	head -c $n /dev/zero | tr '\000' x > ln
	printf '\tz' >> ln
	want=$((n + 8 - n % 8 + 1))
	eq "expand: a tab after $n characters with no newline" "$want" "$("$OURS/expand" ln | wc -c | tr -d ' ')"
	eq "expand: the same from a pipe" "$want" "$(cat ln | "$OURS/expand" | wc -c | tr -d ' ')"
done
LC_ALL=en_US.UTF-8
if [ "$(locale charmap 2> /dev/null)" = UTF-8 ]; then
	# a character split across two reads: two columns wide, so six spaces
	eq "expand: a UTF-8 character cut by the end of a read" "e697a5202020202020780a" \
		"$({ printf '\346\227'; sleep 1; printf '\245\tx\n'; } | "$OURS/expand" | hex)"
	eq "expand: a CJK character is two columns" "e697a5202020202020780a" "$(printf '\346\227\245\tx\n' | "$OURS/expand" | hex)"
	eq "expand: a combining mark is no column" "65cc812020202020202078" "$(printf 'e\314\201\tx' | "$OURS/expand" | hex)"
	eq "expand: an invalid byte takes one column" "ff2020202020202078" "$(printf '\377\tx' | "$OURS/expand" | hex)"
	# unexpand: a column count with wide characters
	eq "unexpand -a: a CJK character is two columns" "e697a509780a" "$(printf '\346\227\245      x\n' | "$OURS/unexpand" -a | hex)"
	eq "unexpand -a: a combining mark is no column" "65cc8109780a" "$(printf 'e\314\201       x\n' | "$OURS/unexpand" -a | hex)"
	eq "unexpand: a blank run cut by the end of a read" "09780a" "$({ printf '    '; sleep 1; printf '    x\n'; } | "$OURS/unexpand" | hex)"
	eq "unexpand: a character cut by the end of a read" "e697a509780a" "$({ printf '\346\227'; sleep 1; printf '\245      x\n'; } | "$OURS/unexpand" -a | hex)"
else
	echo "skip: no en_US.UTF-8 locale"
fi
LC_ALL=C

# unexpand
eq "unexpand: a run held back across reads" "6109620a" "$({ printf 'a  '; sleep 1; printf '     b\n'; } | "$OURS/unexpand" -a | hex)"
eq "unexpand: the last line has no newline" "09" "$(printf '        ' | "$OURS/unexpand" | hex)"
eq "unexpand: a run of seven stays" "20202020202020" "$(printf '       ' | "$OURS/unexpand" | hex)"
eq "unexpand: a run goes on between files" "09780a" "$(printf '       ' > f1; printf ' x\n' > f2; "$OURS/unexpand" f1 f2 | hex)"
eq "unexpand: 100000 lines" 300000 "$("$OURS/unexpand" bigspaces | wc -c | tr -d ' ')"
eq "unexpand -a -t 2: the -a changes nothing" "$(printf 'a   b\n' | "$OURS/unexpand" -t 2 | hex)" "$(printf 'a   b\n' | "$OURS/unexpand" -a -t 2 | hex)"
eq "unexpand -t 2: every run is converted" "610909620a" "$(printf 'a   b\n' | "$OURS/unexpand" -t 2 | hex)"
eq "unexpand: only the leading blanks by default" "$(printf 'a        b\n' | hex)" "$(printf 'a        b\n' | "$OURS/unexpand" | hex)"

# round trip: unexpand then expand gives back the text, for a line of mixed blanks
printf 'x  y\t \t z      w\n        \t    q   \n' > mixed
for opt in "" "-a" "-t 4" "-t 3,9" "-t 2 -a"; do
	"$OURS/unexpand" $opt mixed | "$OURS/expand" $(echo "$opt" | sed 's/-a//') > back
	"$OURS/expand" $(echo "$opt" | sed 's/-a//') mixed > want
	cmp -s back want
	eq "unexpand [$opt] then expand: same columns" 0 $?
done

#------------------------------------------------------- uuencode uudecode
fresh
head -c 300 /dev/urandom > rnd
for algo in "" "-m"; do
	for n in 0 1 2 3 4 5 43 44 45 46 47 56 57 58 59 90 91 100 128 129 299 300; do
		if [ $n -eq 0 ]; then : > in; else head -c $n rnd > in; fi
		"$OURS/uuencode" $algo in name > enc 2> /dev/null
		eq "uuencode $algo of $n bytes: status" 0 $?
		"$OURS/uudecode" -o out enc 2> /dev/null
		eq "uudecode $algo of $n bytes: status" 0 $?
		cmp -s in out
		eq "uuencode $algo | uudecode, $n bytes: identical" 0 $?
		"$OURS/uuencode" $algo name < in > enc2
		"$OURS/uudecode" -o out2 < enc2
		cmp -s in out2
		eq "uuencode $algo | uudecode from stdin, $n bytes: identical" 0 $?
	done
	for n in 65535 65536 65537 100000 1000000; do
		head -c $n /dev/urandom > in
		"$OURS/uuencode" $algo in name | "$OURS/uudecode" -o out
		cmp -s in out
		eq "uuencode $algo | uudecode, $n bytes through a pipe: identical" 0 $?
	done
done
head -c 5000000 /dev/urandom > in
"$OURS/uuencode" in x | "$OURS/uudecode" -o out
cmp -s in out
eq "uuencode | uudecode, 5MB: identical" 0 $?
"$OURS/uuencode" -m in x | "$OURS/uudecode" -o out
cmp -s in out
eq "uuencode -m | uudecode, 5MB: identical" 0 $?

# the layout of the output
printf 'abc' > in
eq "uuencode: layout" "$(printf 'begin 644 n\n#86)C\n \nend\n' | hex)" "$("$OURS/uuencode" in n | hex)"
eq "uuencode: first line" "begin 644 n" "$("$OURS/uuencode" in n | head -1)"
eq "uuencode: last line" "end" "$("$OURS/uuencode" in n | tail -1)"
eq "uuencode -m: first line" "begin-base64 644 n" "$("$OURS/uuencode" -m in n | head -1)"
eq "uuencode -m: last line" "====" "$("$OURS/uuencode" -m in n | tail -1)"
head -c 1000 /dev/urandom > in
eq "uuencode: no line is longer than 61 characters" 0 "$("$OURS/uuencode" in n | awk 'length > 61' | wc -l | tr -d ' ')"
eq "uuencode -m: no line is longer than 76 characters" 0 "$("$OURS/uuencode" -m in n | awk 'length > 76' | wc -l | tr -d ' ')"
eq "uuencode: only characters 0x20 to 0x5f in the body" 0 "$("$OURS/uuencode" in n | sed '1d;$d' | LC_ALL=C tr -d ' -_\n' | wc -c | tr -d ' ')"

# modes
printf data > in
chmod 640 in
eq "uuencode of a file: its mode" "begin 640 n" "$("$OURS/uuencode" in n | head -1)"
eq "uuencode of standard input, a file: its mode" "begin 640 n" "$("$OURS/uuencode" n < in | head -1)"
eq "uuencode of a pipe: 0666 less the umask" "begin 644 n" "$(cat in | "$OURS/uuencode" n | head -1)"
eq "uuencode of a pipe under umask 077" "begin 600 n" "$(umask 077; cat in | "$OURS/uuencode" n | head -1)"
chmod 4755 in
eq "uuencode: permission bits only" "begin 755 n" "$("$OURS/uuencode" in n | head -1)"
chmod 640 in
"$OURS/uuencode" in dec > enc
(umask 077; "$OURS/uudecode" enc)
eq "uudecode: the mode of the data, not the umask" "-rw-r-----" "$(mode dec)"
rm -f dec
printf 'begin 777 dec\n#86)C\n \nend\n' > enc
(umask 777; "$OURS/uudecode" enc)
eq "uudecode: mode 777 under umask 777" "-rwxrwxrwx" "$(mode dec)"
printf 'old\n' > dec
chmod 600 dec
"$OURS/uudecode" enc
eq "uudecode over an existing file: contents" abc "$(cat dec)"
eq "uudecode over an existing file: the mode of the data" "-rwxrwxrwx" "$(mode dec)"
printf 'begin 4755 dec\n#86)C\n \nend\n' > enc
rm -f dec
"$OURS/uudecode" enc
eq "uudecode: no set-user-ID bit" "-rwxr-xr-x" "$(mode dec)"
printf 'begin u=rw,go=r dec\n#86)C\n \nend\n' > enc
rm -f dec
"$OURS/uudecode" enc
eq "uudecode: a symbolic mode" "-rw-r--r--" "$(mode dec)"

# files that cannot be written
if [ "$(id -u)" != 0 ]; then
	printf 'keep\n' > ro
	chmod 444 ro
	printf 'begin 644 ro\n#86)C\n \nend\n' > enc
	"$OURS/uudecode" enc 2> err
	eq "uudecode over a read-only file: status" 1 $?
	eq "uudecode over a read-only file: untouched" keep "$(cat ro)"
	eq "uudecode over a read-only file: diagnostic" 1 "$(wc -l < err | tr -d ' ')"
fi
printf 'begin 644 nodir/x\n#86)C\n \nend\n' > enc
"$OURS/uudecode" enc 2> err
eq "uudecode into a missing directory: status" 1 $?
mkdir dd
"$OURS/uudecode" -o dd enc 2> err
eq "uudecode onto a directory: status" 1 $?

# the standard output
printf 'begin 644 /dev/stdout\n#86)C\n \nend\n' > enc
eq "uudecode: /dev/stdout in the data" abc "$("$OURS/uudecode" enc)"
printf 'begin 644 -\n#86)C\n \nend\n' > enc
eq "uudecode: - in the data" abc "$("$OURS/uudecode" enc)"
printf 'begin 644 other\n#86)C\n \nend\n' > enc
eq "uudecode -o /dev/stdout" abc "$("$OURS/uudecode" -o /dev/stdout enc)"
eq "uudecode -o /dev/stdout: no file made" 0 "$([ -e other ] && echo 1 || echo 0)"
rm -f -- ./-
"$OURS/uudecode" -o - enc
eq "uudecode -o -: a file named -" abc "$(cat ./-)"
rm -f -- ./-
eq "uudecode from -" abc "$("$OURS/uudecode" -o /dev/stdout - < enc)"
eq "uudecode from a pipe" abc "$(cat enc | "$OURS/uudecode" -o /dev/stdout)"
"$OURS/uudecode" -o /dev/stdout enc >&- 2> err
eq "uudecode with stdout closed: status" 1 $?
eq "uudecode with stdout closed: diagnostic" 1 "$(wc -l < err | tr -d ' ')"
head -c 3000000 /dev/urandom > in
"$OURS/uuencode" in big > enc
{ "$OURS/uudecode" -o /dev/stdout enc; echo $? > st; } | head -c 10 > /dev/null
eq "uudecode into a closed pipe dies of SIGPIPE" 141 "$(cat st)"
(trap '' PIPE; { "$OURS/uudecode" -o /dev/stdout enc 2> err; echo $? > st; } | head -c 10 > /dev/null)
eq "uudecode into a closed pipe with SIGPIPE ignored: status" 1 "$(cat st)"
{ "$OURS/uuencode" in big; echo $? > st; } | head -c 10 > /dev/null
eq "uuencode into a closed pipe dies of SIGPIPE" 141 "$(cat st)"
(trap '' PIPE; { "$OURS/uuencode" in big 2> err; echo $? > st; } | head -c 10 > /dev/null)
eq "uuencode into a closed pipe with SIGPIPE ignored: status" 1 "$(cat st)"
"$OURS/uuencode" in big >&- 2> err
eq "uuencode with stdout closed: status" 1 $?
"$OURS/uuencode" -m in big >&- 2> err
eq "uuencode -m with stdout closed: status" 1 $?

# errors
"$OURS/uuencode" nosuch n > o 2> err
eq "uuencode of a missing file: status, no output, a diagnostic" "1 0 1" "$? $(wc -c < o | tr -d ' ') $(wc -l < err | tr -d ' ')"
chmod 000 in
if [ "$(id -u)" != 0 ]; then
	"$OURS/uuencode" in n > o 2> err
	eq "uuencode of an unreadable file: status, no output" "1 0" "$? $(wc -c < o | tr -d ' ')"
fi
chmod 644 in
"$OURS/uuencode" 2> /dev/null
eq "uuencode without operands: usage, status 2" 2 $?
"$OURS/uuencode" in a b 2> /dev/null
eq "uuencode with three operands: status 2" 2 $?
"$OURS/uuencode" in -m > o
eq "uuencode in -m: -m is the name" "begin 644 -m" "$(head -1 o)"
"$OURS/uuencode" in -- > o
eq "uuencode in --: -- is the name" "begin 644 --" "$(head -1 o)"
"$OURS/uudecode" nosuch 2> err
eq "uudecode of a missing file: status" 1 $?
"$OURS/uudecode" a b 2> /dev/null
eq "uudecode with two operands: status 2" 2 $?
"$OURS/uudecode" -o 2> /dev/null
eq "uudecode -o without an argument: status 2" 2 $?
printf 'nothing to see\n' > enc
"$OURS/uudecode" enc 2> err
eq "uudecode without a begin line: status" 1 $?
eq "uudecode without a begin line: diagnostic" 1 "$(wc -l < err | tr -d ' ')"
printf 'begin 644 part\n#86)C\n' > enc
rm -f part
"$OURS/uudecode" enc 2> err
eq "uudecode without an end line: status" 1 $?
eq "uudecode without an end line: the data so far is kept" abc "$(cat part)"
printf 'begin-base64 644 part\nYWJj\nZGVm\n' > enc
rm -f part
"$OURS/uudecode" enc 2> err
eq "uudecode -m data without ====: status" 1 $?
eq "uudecode -m data without ====: the data so far is kept" abcdef "$(cat part)"

# a header after a very long line of text
{ head -c 3000000 /dev/zero | tr '\000' x; printf '\nbegin 644 late\n#86)C\n \nend\n'; } > enc
rm -f late
"$OURS/uudecode" enc
eq "uudecode finds a header after a 3MB line" abc "$(cat late)"
# only the first encoded file in the input is decoded
printf 'begin 644 f1\n#86)C\n \nend\nbegin 644 f2\n#86)C\n \nend\n' > enc
rm -f f1 f2
"$OURS/uudecode" enc
eq "uudecode: only the first file" "abc 0" "$(cat f1) $([ -e f2 ] && echo 1 || echo 0)"

#---------------------------------------------------------------------- comm
fresh
printf 'a\nb\nd\n' > f1
printf 'b\nc\nd\n' > f2
"$OURS/comm" f1 f2 > out
eq "comm: status and the three columns" "0 a/>>b/>c/>>d/" "$? $(tr '\t\n' '>/' < out)"
"$OURS/comm" -123 f1 f2 > out
eq "comm -123: status, no output" "0 0" "$? $(wc -c < out | tr -d ' ')"
"$OURS/comm" f1 f2 f1 2> /dev/null
eq "comm with three operands is a usage error" 2 $?
"$OURS/comm" f1 2> /dev/null
eq "comm with one operand is a usage error" 2 $?
"$OURS/comm" -4 f1 f2 2> /dev/null
eq "comm -4 is a usage error" 2 $?
"$OURS/comm" f1 nosuch > out 2> err
eq "comm of a missing file: status, no output, a diagnostic" "1 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
"$OURS/comm" nosuch f1 2> /dev/null
eq "comm of a missing first file: status" 1 $?
"$OURS/comm" . f1 > out 2> err
eq "comm of a directory: status and a diagnostic" "1 1" "$? $(wc -l < err | tr -d ' ')"
printf 'a\nb\nd\n' | "$OURS/comm" -12 - f2 > out
eq "comm - reads standard input as file1" "b d" "$(tr '\n' ' ' < out | sed 's/ $//')"
printf 'b\nc\nd\n' | "$OURS/comm" -12 f1 - > out
eq "comm - reads standard input as file2" "b d" "$(tr '\n' ' ' < out | sed 's/ $//')"
"$OURS/comm" f1 f2 >&- 2> /dev/null
eq "comm with stdout closed: status" 1 $?
"$OURS/comm" - f1 <&- 2> /dev/null
eq "comm with stdin closed: status" 1 $?
awk 'BEGIN { for (i = 0; i < 200000; i++) printf "%07d\n", i }' > big1
awk 'BEGIN { for (i = 0; i < 200000; i++) printf "%07d\n", i + 100000 }' > big2
"$OURS/comm" -12 big1 big2 > out
eq "comm -12 of two big files: the common 100000 lines" 100000 "$(wc -l < out | tr -d ' ')"
"$OURS/comm" big1 big2 | head -n 2 > out
eq "comm into a closed pipe ends quietly" "0000000 0000001" "$(tr '\n' ' ' < out | sed 's/ $//')"

#---------------------------------------------------------------------- cmp
fresh
printf 'abcd\nefgh\n' > a
printf 'abcd\nefXh\n' > b
cp a a2
"$OURS/cmp" a a2 > out 2> err
eq "cmp of equal files: status, silence" "0 0 0" "$? $(wc -c < out | tr -d ' ') $(wc -c < err | tr -d ' ')"
"$OURS/cmp" a b > out 2> err
eq "cmp of different files: status, report, silence" "1 a b differ: char 8, line 2 0" "$? $(cat out) $(wc -c < err | tr -d ' ')"
"$OURS/cmp" -s a b > out 2> err
eq "cmp -s of different files: status, silence" "1 0 0" "$? $(wc -c < out | tr -d ' ') $(wc -c < err | tr -d ' ')"
"$OURS/cmp" -l a b > out 2> err
eq "cmp -l: status and list" "1 8 147 130" "$? $(tr -s ' ' < out | sed 's/^ //')"
printf 'abcd\nef' > pre
"$OURS/cmp" pre a > out 2> err
eq "cmp of a prefix: status, nothing on stdout" "1 0" "$? $(wc -c < out | tr -d ' ')"
eq "cmp of a prefix: EOF on the shorter file, one line" "cmp: EOF on pre" "$(sed 's/ .*after.*//' err)"
eq "cmp of a prefix: one line" 1 "$(wc -l < err | tr -d ' ')"
"$OURS/cmp" -s pre a > out 2> err
eq "cmp -s of a prefix: status, silence" "1 0 0" "$? $(wc -c < out | tr -d ' ') $(wc -c < err | tr -d ' ')"
"$OURS/cmp" -l -s a b 2> /dev/null
eq "cmp -l -s is a usage error" 2 $?
"$OURS/cmp" a 2> /dev/null
eq "cmp with one operand is a usage error" 2 $?
"$OURS/cmp" a b a 2> /dev/null
eq "cmp with three operands is a usage error" 2 $?
"$OURS/cmp" a nosuch > out 2> err
eq "cmp of a missing file: status 2, a diagnostic" "2 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
"$OURS/cmp" . a 2> err
eq "cmp of a directory: status 2 and a diagnostic" "2 1" "$? $(wc -l < err | tr -d ' ')"
"$OURS/cmp" a b >&- 2> /dev/null
eq "cmp with stdout closed: status 2" 2 $?
"$OURS/cmp" - a <&- 2> /dev/null
eq "cmp with stdin closed: status 2" 2 $?
printf 'abcd\nefgh\n' | "$OURS/cmp" - a
eq "cmp - reads standard input as file1" 0 $?
printf 'abcd\nefgh\n' | "$OURS/cmp" a -
eq "cmp - reads standard input as file2" 0 $?
# GA34: stdin that can seek is left just past the last byte looked at
printf 'xyz' > s1
printf 'abXdef' > s2
printf 'abYdef' > s3
( "$OURS/cmp" s2 - > /dev/null; cat ) < s3 > out
eq "cmp leaves stdin just past the first differing byte" "def" "$(cat out)"
( "$OURS/cmp" -s s2 - ; cat ) < s3 > out
eq "cmp -s leaves stdin just past the first differing byte" "def" "$(cat out)"
printf 'ab' > p1
( "$OURS/cmp" p1 - 2> /dev/null; cat ) < s2 > out
eq "cmp of a prefix leaves stdin just past the shorter length" "Xdef" "$(cat out)"
( "$OURS/cmp" -l p1 - 2> /dev/null; cat ) < s2 > out
eq "cmp -l of a prefix leaves stdin just past the shorter length" "Xdef" "$(cat out)"
# a big file: the difference is past several blocks, and the line count is file1's
head -c 3000000 /dev/zero | tr '\000' '\n' > n1
{ head -c 2000000 /dev/zero | tr '\000' '\n'; printf x; head -c 999999 /dev/zero | tr '\000' '\n'; } > n2
"$OURS/cmp" n1 n2 > out
eq "cmp of big files: byte and line" "n1 n2 differ: char 2000001, line 2000001" "$(cat out)"
head -c 3000000 /dev/zero > z1
cp z1 z2
"$OURS/cmp" z1 z2
eq "cmp of big equal files" 0 $?
"$OURS/cmp" -l n1 n2 | head -n 1 > out
eq "cmp into a closed pipe ends quietly" "2000001 12 170" "$(tr -s ' ' < out | sed 's/^ //')"

# cksum: standard input, descriptors, a stream that arrives in pieces
printf a > ck1
seq 1 300000 > big
eq "cksum: a known file" "1220704766 1 ck1" "$("$OURS/cksum" ck1)"
eq "cksum of standard input has no name" "1220704766 1" "$("$OURS/cksum" < ck1)"
eq "cksum - is standard input and keeps its name" "1220704766 1 -" "$("$OURS/cksum" - < ck1)"
"$OURS/cksum" <&- > out 2> err
eq "cksum with stdin closed: status 1, a diagnostic, no output" "1 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
"$OURS/cksum" - <&- > out 2> err
eq "cksum - with stdin closed: status 1, a diagnostic, no output" "1 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
"$OURS/cksum" ck1 >&- 2> /dev/null
eq "cksum with stdout closed: status 1" 1 $?
"$OURS/cksum" . ck1 > out 2> err
eq "cksum of a directory then a file: both reported" "1 1220704766 1 ck1 1" "$? $(cat out) $(wc -l < err | tr -d ' ')"
eq "cksum of a big file through a pipe equals the file" "$("$OURS/cksum" big | sed 's/ big$//')" "$(cat big | "$OURS/cksum")"
eq "cksum of a big file arriving in 1000-byte pieces" "$("$OURS/cksum" big | sed 's/ big$//')" "$(dd if=big bs=1000 2> /dev/null | "$OURS/cksum")"

# id: the same numbers whichever way they are asked for, descriptors
eq "id -u is the number of the id line" "$("$OURS/id" | sed 's/^uid=\([0-9]*\).*/\1/')" "$("$OURS/id" -r -u)"
eq "id -u -n is the name in the id line" "$("$OURS/id" | sed 's/^uid=[0-9]*(\([^)]*\)).*/\1/')" "$("$OURS/id" -u -n)"
eq "id -g -n is the name after gid=" "$("$OURS/id" | sed 's/^uid=[0-9]*([^)]*) gid=[0-9]*(\([^)]*\)).*/\1/')" "$("$OURS/id" -r -g -n)"
eq "id -G and id -G -n list the same number of groups" "$("$OURS/id" -G | wc -w | tr -d ' ')" "$("$OURS/id" -G -n | wc -w | tr -d ' ')"
eq "id root starts with the id of root" "uid=0(root) gid=0(" "$("$OURS/id" root | cut -c1-18)"
eq "id -u root" 0 "$("$OURS/id" -u root)"
"$OURS/id" nosuch > out 2> err
eq "id of an unknown user: status 1, a diagnostic, no output" "1 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
"$OURS/id" -u >&- 2> /dev/null
eq "id with stdout closed: status 1" 1 $?
"$OURS/id" -n 2> /dev/null
eq "id -n alone is a usage error" 2 $?
"$OURS/id" -G -g 2> /dev/null
eq "id -G -g is a usage error" 2 $?
"$OURS/id" a b 2> /dev/null
eq "id with two operands is a usage error" 2 $?
eq "id works with standard input closed" 0 "$("$OURS/id" -u <&- > /dev/null 2>&1; echo $?)"

#----------------------------------------------------- paste fold cut
seq 1 5000 > pbig
printf '1\n2\n3\n' > p1
printf 'a\nb\n' > p2
eq "paste: two files side by side" "1 a|2 b|3 " "$("$OURS/paste" -d ' ' p1 p2 | tr '\n' '|' | sed 's/|$//')"
eq "paste -s: lines of a file on one line" "1,2,3" "$("$OURS/paste" -s -d, p1)"
eq "paste: the standard input is shared by its \"-\" operands" "1 2|3 " "$("$OURS/paste" -d ' ' - - < p1 | tr '\n' '|' | sed 's/|$//')"
eq "paste of one file is the file" 0 "$("$OURS/paste" pbig | cmp - pbig > /dev/null; echo $?)"
eq "paste of one file through a pipe is the file" 0 "$(cat pbig | "$OURS/paste" - | cmp - pbig > /dev/null; echo $?)"
eq "paste - - of a pipe: half as many lines" 2500 "$(cat pbig | "$OURS/paste" - - | wc -l | tr -d ' ')"
eq "paste -s of a big file: one line" 1 "$("$OURS/paste" -s pbig | wc -l | tr -d ' ')"
"$OURS/paste" p1 nosuch > out 2> err
eq "paste with a file that cannot be opened: status 1, a diagnostic, no output" "1 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
eq "paste -s goes on after a file that cannot be opened" "1 6109620a6109620a 1" "$("$OURS/paste" -s p2 nosuch p2 > out 2> err; echo $?) $(hex < out) $(wc -l < err | tr -d ' ')"
"$OURS/paste" p1 >&- 2> /dev/null
eq "paste with stdout closed: status 1" 1 $?
"$OURS/paste" - <&- > out 2> err
eq "paste - with stdin closed: status 1, a diagnostic" "1 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
"$OURS/paste" 2> /dev/null
eq "paste with no operand is a usage error" 2 $?
"$OURS/paste" -d '' p1 2> /dev/null
eq "paste -d '' is an error" 1 $?
"$OURS/paste" -d 'a\' p1 2> /dev/null
eq "paste -d with a lone backslash at the end is an error" 1 $?
seq 1 300000 > pmax
{ "$OURS/paste" pmax pmax; echo $? > st; } | head -c 10 > /dev/null
eq "paste into a closed pipe ends quietly" 0 "$([ "$(cat st)" -ne 0 ] && echo 0 || echo 1)"
start=$(date +%s)
{ printf 'a\nb\n'; sleep 3; } | "$OURS/paste" - - | { read -r line; date +%s > t1; }
eq "paste answers before its input ends" 1 "$([ $(($(cat t1) - start)) -lt 3 ] && echo 1 || echo 0)"

# fold
eq "fold: lines of 80 columns by default" "80 80 40" "$(head -c 200 /dev/zero | tr '\000' x | "$OURS/fold" | awk '{ printf "%s%d", s, length($0); s = " " }')"
eq "fold -w 5 -s: after the last blank within the width" "ab |cd ef" "$(printf 'ab cd ef\n' | "$OURS/fold" -s -w 5 | tr '\n' '|' | sed 's/|$//')"
eq "fold: a last line without a newline stays without" "61620a6364" "$(printf 'abcd' | "$OURS/fold" -w 2 | hex)"
eq "fold of a big file with a width that fits is the file" 0 "$("$OURS/fold" -w 100 pbig | cmp - pbig > /dev/null; echo $?)"
eq "fold of a big pipe is the same as of the file" 0 "$(cat pbig | "$OURS/fold" -w 2 | cmp - "$("$OURS/fold" -w 2 pbig > fo; echo fo)" > /dev/null; echo $?)"
head -c 300000 /dev/zero | tr '\000' x > fl
eq "fold: a line of 300000 bytes (no newline) in 7-byte pieces" 42857 "$("$OURS/fold" -b -w 7 fl | wc -l | tr -d ' ')"
eq "fold: -w 18446744073709551615 never folds" 300000 "$("$OURS/fold" -w 18446744073709551615 fl | wc -c | tr -d ' ')"
"$OURS/fold" -w 0 pbig > out 2> err
eq "fold -w 0: status 1, a diagnostic, no output" "1 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
"$OURS/fold" pbig >&- 2> /dev/null
eq "fold with stdout closed: status 1" 1 $?
"$OURS/fold" <&- > out 2> err
eq "fold with stdin closed: status 1, a diagnostic" "1 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
"$OURS/fold" nosuch p1 > out 2> err
eq "fold goes on after a file that cannot be opened" "1 6 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
{ "$OURS/fold" -w 1 fl; echo $? > st; } | head -c 10 > /dev/null
eq "fold into a closed pipe ends quietly" 0 "$([ "$(cat st)" -ne 0 ] && echo 0 || echo 1)"
start=$(date +%s)
{ printf 'abcdef\n'; sleep 3; } | "$OURS/fold" -w 3 | { read -r line; date +%s > t1; }
eq "fold answers before its input ends" 1 "$([ $(($(cat t1) - start)) -lt 3 ] && echo 1 || echo 0)"
if [ "$(LC_ALL=en_US.UTF-8 locale charmap 2> /dev/null)" = UTF-8 ]; then
	eq "fold: a UTF-8 character cut by the end of a read stays whole" "e697a50a65" "$({ printf '\346\227'; sleep 1; printf '\245e\n'; } | LC_ALL=en_US.UTF-8 "$OURS/fold" -w 2 | hex | cut -c1-10)"
	eq "fold: two CJK characters need four columns" "e697a50ae697a5" "$(printf '\346\227\245\346\227\245' | LC_ALL=en_US.UTF-8 "$OURS/fold" -w 3 | hex)"
fi

# cut
awk 'BEGIN { for (i = 1; i <= 5000; i++) printf "%d:%d:%d\n", i, i * 7, i % 13 }' > cf
eq "cut -b against awk substr" 0 "$("$OURS/cut" -b 2-3 pbig | cmp - "$(awk '{ print substr($0, 2, 2) }' pbig > aw; echo aw)" > /dev/null; echo $?)"
eq "cut -f against awk -F:" 0 "$("$OURS/cut" -d: -f2 cf | cmp - "$(awk -F: '{ print $2 }' cf > aw; echo aw)" > /dev/null; echo $?)"
eq "cut -f 3,1 -d: is in the order of the input" 0 "$("$OURS/cut" -d: -f3,1 cf | cmp - "$(awk -F: '{ print $1 ":" $3 }' cf > aw; echo aw)" > /dev/null; echo $?)"
eq "cut -c of a pipe equals cut -c of the file" 0 "$(cat cf | "$OURS/cut" -c 2-5 | cmp - "$("$OURS/cut" -c 2-5 cf > cu; echo cu)" > /dev/null; echo $?)"
eq "cut: a last line without a newline gets one" "610a" "$(printf 'ab' | "$OURS/cut" -b 1 | hex)"
eq "cut -s drops a line with no delimiter" "620a" "$(printf 'a\nx:b\n' | "$OURS/cut" -s -d: -f2 | hex)"
eq "cut -f keeps a line with no delimiter" "610a620a" "$(printf 'a\nx:b\n' | "$OURS/cut" -d: -f2 | hex)"
eq "cut -n: a start inside a character moves back" "c3a90a" "$(printf '\303\251\n' | LC_ALL=en_US.UTF-8 "$OURS/cut" -n -b 2 | hex)"
eq "cut -n: an end inside a character drops the character" "0a" "$(printf '\303\251\n' | LC_ALL=en_US.UTF-8 "$OURS/cut" -n -b 1 | hex)"
eq "cut -b may cut a character in two" "c30a" "$(printf '\303\251\n' | LC_ALL=en_US.UTF-8 "$OURS/cut" -b 1 | hex)"
eq "cut -c takes a whole character" "c3a90a" "$(printf '\303\251\n' | LC_ALL=en_US.UTF-8 "$OURS/cut" -c 1 | hex)"
head -c 300000 /dev/zero | tr '\000' x > cl
echo >> cl
eq "cut: a line of 300000 bytes" 100 "$("$OURS/cut" -b 299901-300000 cl | tr -d '\n' | wc -c | tr -d ' ')"
eq "cut -f of a line of 300000 bytes" 300001 "$("$OURS/cut" -d: -f1 cl | wc -c | tr -d ' ')"
"$OURS/cut" -b 0 p1 > out 2> err
eq "cut -b 0: status 1, a diagnostic, no output" "1 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
"$OURS/cut" -b 3-1 p1 2> /dev/null
eq "cut -b 3-1 (a range that runs backwards) is an error" 1 $?
"$OURS/cut" p1 2> /dev/null
eq "cut without -b -c -f is a usage error" 2 $?
"$OURS/cut" -b 1 -c 1 p1 2> /dev/null
eq "cut -b -c is a usage error" 2 $?
"$OURS/cut" -c 1 -n p1 2> /dev/null
eq "cut -c -n is a usage error" 2 $?
"$OURS/cut" -b 1 -d: p1 2> /dev/null
eq "cut -b -d is a usage error" 2 $?
"$OURS/cut" -f 1 -d ab p1 2> /dev/null
eq "cut -d with two characters is an error" 1 $?
"$OURS/cut" -b 1 p1 nosuch p2 > out 2> err; st=$?
eq "cut goes on after a file that cannot be opened" "1 1 2 3 a b 1" "$st $(tr '\n' ' ' < out | sed 's/ $//') $(wc -l < err | tr -d ' ')"
"$OURS/cut" -b 1 pbig >&- 2> /dev/null
eq "cut with stdout closed: status 1" 1 $?
"$OURS/cut" -b 1 <&- > out 2> err
eq "cut with stdin closed: status 1, a diagnostic" "1 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
{ "$OURS/cut" -b 1 pbig pbig pbig pbig pbig pbig pbig pbig pbig pbig; echo $? > st; } | head -c 10 > /dev/null
eq "cut into a closed pipe ends quietly" 0 "$([ "$(cat st)" -ne 0 ] && echo 0 || echo 1)"
start=$(date +%s)
{ printf 'abc\n'; sleep 3; } | "$OURS/cut" -b 1 | { read -r line; date +%s > t1; }
eq "cut answers before its input ends" 1 "$([ $(($(cat t1) - start)) -lt 3 ] && echo 1 || echo 0)"

# GA39: output that cannot be written (a closed pipe, SIGPIPE ignored) gets a
# diagnostic on standard error and a status other than 0 (PCTS 801)
gaw() {
	rm -f st err
	( trap '' PIPE; sleep 1; eval "$2" 2> err; echo $? > st ) | true
	eq "$1: a closed pipe with SIGPIPE ignored: diagnostic and non-zero status" "1 1" \
		"$([ "$(cat st)" -ne 0 ] && echo 1 || echo 0) $([ -s err ] && echo 1 || echo 0)"
}
gaw cksum 'echo x | "$OURS/cksum"'
gaw id '"$OURS/id"'
gaw paste 'echo x | "$OURS/paste" -'
gaw fold 'echo x | "$OURS/fold"'
gaw cut 'echo x | "$OURS/cut" -f 1'

#---------------------------------------------------------------------- rm
# Everything that could reach the root is run with -i and an answer of "n", or
# without -r, so that a bug could not delete anything.
fresh
echo x > f
echo y | "$OURS/rm" -i f > o 2> e
eq "rm -i: the prompt goes to stderr and names the file" "0 0 1" "$? $(wc -c < o | tr -d ' ') $(grep -c 'f' e)"
eq "rm -i: an affirmative answer removes the file" gone "$([ -e f ] || echo gone)"
echo x > f
echo n | "$OURS/rm" -i f 2> /dev/null
eq "rm -i: a negative answer keeps the file, status 0" "0 kept" "$? $([ -e f ] && echo kept)"
"$OURS/rm" -i f < /dev/null 2> /dev/null
eq "rm -i: end of file is not an affirmative answer" "0 kept" "$? $([ -e f ] && echo kept)"
"$OURS/rm" -i f <&- 2> /dev/null
eq "rm -i: a closed standard input is not an affirmative answer" "0 kept" "$? $([ -e f ] && echo kept)"
printf 'Yes\n' | "$OURS/rm" -i f 2> /dev/null
eq "rm -i: Yes is affirmative (the C locale yesexpr is ^[yY])" gone "$([ -e f ] || echo gone)"
echo x > f
printf 'maybe\n' | "$OURS/rm" -i f 2> /dev/null
eq "rm -i: anything else is not" kept "$([ -e f ] && echo kept)"
echo x > f; echo x > g
printf ' y\n' | "$OURS/rm" -i f 2> /dev/null
eq "rm -i: a leading blank spoils the answer" kept "$([ -e f ] && echo kept)"
rm -f f g
echo x > f
echo x > g
eq "rm -i reads only its line: the rest of a shared standard input is left" "REST more" "$(printf 'y\nREST\nmore\n' | { "$OURS/rm" -i g 2> /dev/null; cat; } | tr '\n' ' ' | sed 's/ $//')"
echo x > f; echo x > g
printf 'n\ny\n' | "$OURS/rm" -i f g 2> /dev/null
eq "rm -i f g: an answer for each operand" "0 kept gone" "$? $([ -e f ] && echo kept) $([ -e g ] || echo gone)"
rm -f f g

mkdir d; echo x > d/a; echo x > d/b
printf 'y\nn\ny\nn\n' | "$OURS/rm" -ri d 2> e
eq "rm -ri: four prompts (directory, two files, directory again), one file survives" "0 4 1" "$? $(grep -o '?' e | wc -l | tr -d ' ') $(ls d | wc -l | tr -d ' ')"
rm -rf d
mkdir d; echo x > d/a
printf 'y\ny\ny\n' | "$OURS/rm" -ri d 2> e
eq "rm -ri: yes to everything removes the tree" "0 gone" "$? $([ -e d ] || echo gone)"
mkdir e1
printf 'n\n' | "$OURS/rm" -ri e1 2> e
eq "rm -ri: an empty directory is asked about once" "1 kept" "$(grep -o '?' e | wc -l | tr -d ' ') $([ -d e1 ] && echo kept)"
rmdir e1

# the root, in every spelling: refused with a diagnostic, nothing removed, the next operand still handled
touch flag
"$OURS/rm" / // /// /. /.. . .. flag 2> e
eq "rm of the root and of dot, without -r: status 1, the flag file is gone" "1 gone" "$? $([ -e flag ] || echo gone)"
touch flag
ln -s / lnk
printf 'n\nn\nn\nn\nn\nn\n' | "$OURS/rm" -ri / // /// lnk/ flag 2> e
eq "rm -ri of the root: no prompt for it (only the flag file is asked about)" "1 1" "$? $(grep -o '?' e | wc -l | tr -d ' ')"
eq "rm -ri of the root: diagnostics" 4 "$(grep -c 'root' e)"
eq "rm -ri of the root: nothing was asked about but the flag file" 1 "$(grep -c 'flag' e)"
rm -f lnk flag

# a link with a trailing slash is not walked through
mkdir real; echo x > real/f; ln -s real lnk
"$OURS/rm" -r lnk/ 2> e
eq "rm -r link/: refused, the target is untouched" "1 kept kept" "$? $([ -e real/f ] && echo kept) $([ -h lnk ] && echo kept)"
"$OURS/rm" -r lnk 2> /dev/null
eq "rm -r link: removes the link only" "0 kept gone" "$? $([ -e real/f ] && echo kept) $([ -h lnk ] || echo gone)"
rm -rf real

# depth: a tree deeper than any pathname can be, and with very few descriptors
mkdir deep
(
	cd deep || exit 1
	i=0
	while [ $i -lt 700 ]; do
		mkdir dddddddddddddddd || exit 1
		cd dddddddddddddddd || exit 1
		: > file
		i=$((i + 1))
	done
)
"$OURS/rm" -r deep 2> e
eq "rm -r of a tree 700 levels deep (over 11000 bytes of path)" "0 gone" "$? $([ -e deep ] || echo gone)"
mkdir deep
(
	cd deep || exit 1
	i=0
	while [ $i -lt 300 ]; do
		mkdir dddddddddddddddd || exit 1
		cd dddddddddddddddd || exit 1
		: > file
		i=$((i + 1))
	done
)
(ulimit -n 12 2> /dev/null && "$OURS/rm" -r deep 2> e; echo $? > st)
eq "rm -r with 12 descriptors allowed, 300 levels deep" "0 gone" "$(cat st) $([ -e deep ] || echo gone)"
rm -rf deep

# a wide directory
mkdir wide
( cd wide && i=0 && while [ $i -lt 3000 ]; do : > "file$i"; i=$((i + 1)); done )
"$OURS/rm" -r wide 2> e
eq "rm -r of a directory with 3000 entries" "0 gone" "$? $([ -e wide ] || echo gone)"

# closed descriptors
echo x > f
"$OURS/rm" f >&-
eq "rm with stdout closed" "0 gone" "$? $([ -e f ] || echo gone)"
"$OURS/rm" nosuch 2>&-
eq "rm of a missing file with stderr closed: still status 1" 1 $?
"$OURS/rm" -f nosuch 2>&-
eq "rm -f of a missing file with stderr closed: status 0" 0 $?

# a failure is reported once and the other operands go on; the exit status is non-zero
mkdir -p keep/sub; echo x > keep/sub/x; echo x > other
chmod 555 keep/sub
"$OURS/rm" -r keep other 2> e
eq "rm: a file that cannot be removed: status 1, the other operand is gone" "1 gone" "$? $([ -e other ] || echo gone)"
eq "rm: one diagnostic for the file, none for the directories that could not be emptied" 1 "$(wc -l < e | tr -d ' ')"
chmod 755 keep/sub
rm -rf keep

# the standard input is not read without -i or a terminal
echo x > f
echo y | "$OURS/rm" f
eq "rm f does not touch standard input" "0 gone" "$? $([ -e f ] || echo gone)"
printf 'y\n' > ans
echo x > f; chmod 444 f
"$OURS/rm" f < ans 2> e
eq "rm of a read-only file with a non-terminal standard input: no prompt" "0 gone 0" "$? $([ -e f ] || echo gone) $(wc -c < e | tr -d ' ')"
rm -f f ans

# prompts on a terminal for what is not writable (script gives a pseudo-terminal)
if [ -x /usr/bin/script ] && /usr/bin/script -q /dev/null true < /dev/null > /dev/null 2>&1 && [ "$(id -u)" != 0 ]; then
	export OURS
	echo x > ro1; echo x > ro2; echo x > rw; chmod 444 ro1 ro2
	{ sleep 1; printf 'n\ny\n'; sleep 1; } | /usr/bin/script -q /dev/null sh -c '"$OURS/rm" rw ro1 ro2; echo "st=$?" > st' > ty 2>&1
	eq "rm on a terminal: asks for the read-only files only, not for a writable one" "1 1 0" "$(grep -c ro1 ty) $(grep -c ro2 ty) $(grep -c "remove 'rw'" ty)"
	eq "rm on a terminal: n keeps, y removes, the writable one is gone unasked" "kept gone gone" "$([ -e ro1 ] && echo kept) $([ -e ro2 ] || echo gone) $([ -e rw ] || echo gone)"
	eq "rm on a terminal: status 0" "st=0" "$(cat st)"
	echo x > ro3; chmod 444 ro3
	{ sleep 1; printf 'y\n'; sleep 1; } | /usr/bin/script -q /dev/null sh -c '"$OURS/rm" -f ro3; echo "st=$?" > st' > ty 2>&1
	eq "rm -f on a terminal: never asks" "gone 0" "$([ -e ro3 ] || echo gone) $(grep -c ro3 ty)"
	rm -f ro1 ro2 rw ro3 st ty
else
	echo "skip: rm on a terminal (no script(1), or running as root)"
fi

#-------------------------------------------------------------------- touch
# mt/at FILE: modification and access time in seconds (BSD stat, else GNU stat)
mt() { stat -f %m "$1" 2> /dev/null || stat -c %Y "$1"; }
at() { stat -f %a "$1" 2> /dev/null || stat -c %X "$1"; }

fresh
"$OURS/touch" new
eq "touch of a missing file creates it empty, status 0" "0 0 file" "$? $(wc -c < new | tr -d ' ') $([ -f new ] && echo file)"
eq "touch: a created file gets 0666 less the umask 022" -rw-r--r-- "$(mode new)"
(umask 077; "$OURS/touch" private)
eq "touch: a created file under umask 077" -rw------- "$(mode private)"
(umask 0; "$OURS/touch" open)
eq "touch: a created file under umask 0" -rw-rw-rw- "$(mode open)"
echo data > keep
"$OURS/touch" keep
eq "touch of an existing file leaves the contents" data "$(cat keep)"
rm -f new private open keep

echo x > f
"$OURS/touch" -d 2001-01-01T00:00:00Z f
before=$(mt f)
"$OURS/touch" f
eq "touch f sets both times to now" "1 1" "$([ "$(mt f)" -gt "$before" ] && echo 1) $([ "$(at f)" -gt "$before" ] && echo 1)"

# -a and -m change one time and leave the other
"$OURS/touch" -d 2001-01-01T00:00:00Z f
"$OURS/touch" -m -d 2002-02-02T00:00:00Z f
eq "touch -m -d: modification only" "978307200 1012608000" "$(at f) $(mt f)"
"$OURS/touch" -a -d 2003-03-03T00:00:00Z f
eq "touch -a -d: access only" "1046649600 1012608000" "$(at f) $(mt f)"
"$OURS/touch" -am -d 2010-10-10T00:00:00Z f
eq "touch -am -d: both" "1286668800 1286668800" "$(at f) $(mt f)"
"$OURS/touch" -d 2001-01-01T00:00:00Z f
"$OURS/touch" -m f
eq "touch -m (now): access time untouched, modification time moved" "978307200 1" "$(at f) $([ "$(mt f)" -gt 1286668800 ] && echo 1)"
"$OURS/touch" -d 2001-01-01T00:00:00Z f
"$OURS/touch" -a f
eq "touch -a (now): modification time untouched, access time moved" "978307200 1" "$(mt f) $([ "$(at f)" -gt 1286668800 ] && echo 1)"

# -r: each time from the reference
echo r > ref
"$OURS/touch" -a -d 2003-03-03T00:00:00Z ref
"$OURS/touch" -m -d 2002-02-02T00:00:00Z ref
echo x > g
"$OURS/touch" -r ref g
eq "touch -r copies both times" "1046649600 1012608000" "$(at g) $(mt g)"
"$OURS/touch" -d 2010-10-10T00:00:00Z g
"$OURS/touch" -a -r ref g
eq "touch -a -r: only the access time is copied" "1046649600 1286668800" "$(at g) $(mt g)"
"$OURS/touch" -d 2010-10-10T00:00:00Z g
"$OURS/touch" -m -r ref g
eq "touch -m -r: only the modification time is copied" "1286668800 1012608000" "$(at g) $(mt g)"
ln -s ref lref
"$OURS/touch" -d 2010-10-10T00:00:00Z g
"$OURS/touch" -r lref g
eq "touch -r follows a symbolic link to the reference" "1046649600 1012608000" "$(at g) $(mt g)"
"$OURS/touch" -d 2010-10-10T00:00:00Z g
"$OURS/touch" -r nosuch g f 2> e
eq "touch -r with a missing reference: status 1, a diagnostic, nothing touched" "1 1 1286668800" "$? $(wc -l < e | tr -d ' ') $(mt g)"
eq "touch -r with a missing reference names it" 1 "$(grep -c nosuch e)"
"$OURS/touch" -c -r nosuch newone 2> /dev/null
eq "touch -c -r nosuch: still an error" "1 absent" "$? $([ -e newone ] || echo absent)"
"$OURS/touch" -r ref newfromref
eq "touch -r creates a missing file with the reference's times" "1046649600 1012608000" "$(at newfromref) $(mt newfromref)"
rm -f g ref lref newfromref

# -t and -d in several zones: what a time without Z means is decided by TZ
for z in UTC0:1194862530 EST5EDT:1194880530 EET-2EEST:1194855330; do
	tz=${z%%:*} want=${z##*:}
	TZ=$tz "$OURS/touch" -t 200711121015.30 f
	eq "touch -t 200711121015.30 under TZ=$tz" "$want $want" "$(mt f) $(at f)"
	TZ=$tz "$OURS/touch" -t 0711121015.30 f
	eq "touch -t 0711121015.30 (two-digit year) under TZ=$tz" "$want" "$(mt f)"
	TZ=$tz "$OURS/touch" -d 2007-11-12T10:15:30 f
	eq "touch -d 2007-11-12T10:15:30 under TZ=$tz" "$want" "$(mt f)"
	TZ=$tz "$OURS/touch" -d '2007-11-12 10:15:30' f
	eq "touch -d with a space under TZ=$tz" "$want" "$(mt f)"
	TZ=$tz "$OURS/touch" -d 2007-11-12T10:15:30Z f
	eq "touch -d ...Z ignores TZ=$tz" 1194862530 "$(mt f)"
done
TZ=EST5EDT "$OURS/touch" -t 200707121015 f
eq "touch -t in summer time under TZ=EST5EDT" 1184249700 "$(mt f)"
TZ=EST5EDT "$OURS/touch" -d 2007-07-12T10:15:00 f
eq "touch -d in summer time under TZ=EST5EDT" 1184249700 "$(mt f)"

# the century rule, the second 60, the Epoch and 2038
TZ=UTC0 "$OURS/touch" -t 6901010000 f
eq "touch -t 6901010000 is 1969" -31536000 "$(mt f)"
TZ=UTC0 "$OURS/touch" -t 6812312359 f
eq "touch -t 6812312359 is 2068" 3124223940 "$(mt f)"
TZ=UTC0 "$OURS/touch" -t 199812312359.60 f
eq "touch -t ....60 is one second after .59" 915148800 "$(mt f)"
"$OURS/touch" -d 1998-12-31T23:59:60Z f
eq "touch -d ...:60Z is one second after :59" 915148800 "$(mt f)"
TZ=UTC0 "$OURS/touch" -t 190001010000 f
eq "touch -t 190001010000: before the Epoch is fine where the file system holds it" -2208988800 "$(mt f)"
"$OURS/touch" -d 1969-12-31T23:59:59Z f
eq "touch -d 1969-12-31T23:59:59Z" -1 "$(mt f)"
TZ=UTC0 "$OURS/touch" -t 203801010000 f
eq "touch -t 203801010000 (the spec's minimum)" 2145916800 "$(mt f)"
TZ=UTC0 "$OURS/touch" -t 210001010000 f
eq "touch -t 210001010000" 4102444800 "$(mt f)"

# fractions of a second
"$OURS/touch" -d 2007-11-12T10:15:30.123456789Z f
if stat -f %Fm f > /dev/null 2>&1; then
	eq "touch -d keeps nanoseconds" 1194862530.123456789 "$(stat -f %Fm f)"
	"$OURS/touch" -d 2007-11-12T10:15:30,5Z f
	eq "touch -d accepts a comma fraction" 1194862530.500000000 "$(stat -f %Fm f)"
	"$OURS/touch" -d 2007-11-12T10:15:30.9999999999Z f
	eq "touch -d cuts a fraction beyond nanoseconds" 1194862530.999999999 "$(stat -f %Fm f)"
else
	echo "skip: touch fractions (no BSD stat)"
fi

# the time cannot be a file timestamp: error, and the file keeps its times
"$OURS/touch" -d 2001-01-01T00:00:00Z f g_untouched
"$OURS/touch" -d 2001-01-01T00:00:00Z g_untouched
"$OURS/touch" -d 99999-01-01T00:00:00Z f g_untouched 2> e
eq "touch: a time no file can hold: status 1" 1 $?
eq "touch: ... a single diagnostic, and it stops (the second file is not looked at)" "1 978307200 978307200" "$(wc -l < e | tr -d ' ') $(mt f) $(mt g_untouched)"
"$OURS/touch" -d 2001-01-01T00:00:00Z newfile_never 2> /dev/null
rm -f newfile_never
"$OURS/touch" -d 99999-01-01T00:00:00Z created_then_refused 2> /dev/null
eq "touch: ... the same for a file it had to create: status 1" 1 $?
rm -f created_then_refused g_untouched

# invalid arguments: a diagnostic, nothing touched, status 2 (usage)
echo x > h
for bad in "-t 13010000" "-t 0101000" "-t 01010000.6" "-d 2007-11-12" "-d 2007-11-12T10:15" "-t abc" "-d ''"; do
	eval "\"\$OURS/touch\" $bad h" 2> e
	eq "touch $bad: status 2 and a diagnostic" "2 1" "$? $([ -s e ] && echo 1)"
done
"$OURS/touch" -r ref -t 200711121015 h 2> /dev/null
eq "touch -r and -t together: usage error" 2 $?
"$OURS/touch" -t 200711121015 -d 2007-11-12T10:15:30Z h 2> /dev/null
eq "touch -t and -d together: usage error" 2 $?
"$OURS/touch" 2> e
eq "touch with no operand: status 2 and a usage line" "2 usage" "$? $(sed 's/:.*//;q' e)"
"$OURS/touch" -a 2> /dev/null
eq "touch -a with no operand: status 2" 2 $?
"$OURS/touch" -r 2> /dev/null
eq "touch -r with no argument: status 2" 2 $?
"$OURS/touch" -x h 2> /dev/null
eq "touch -x: status 2" 2 $?

# -c
"$OURS/touch" -c absent > o 2> e
eq "touch -c of a missing file: status 0, silent, nothing created" "0 0 0 absent" "$? $(wc -c < o | tr -d ' ') $(wc -c < e | tr -d ' ') $([ -e absent ] || echo absent)"
"$OURS/touch" -c nodir/x h 2> e
eq "touch -c nodir/x: silent" "0 0" "$? $(wc -c < e | tr -d ' ')"
"$OURS/touch" -c h
eq "touch -c of an existing file still touches it" 1 "$([ "$(mt h)" -gt 1286668800 ] && echo 1)"

# options are not looked for after the first operand; -- ends them
"$OURS/touch" h -m
eq "touch h -m: -m is a file name" "-m" "$(ls -- -m)"
rm -f -- -m
"$OURS/touch" -- -m
eq "touch -- -m: creates -m" "-m" "$(ls -- -m)"
"$OURS/touch" h --
eq "touch h --: creates a file named --" "made" "$([ -f ./-- ] && echo made)"
rm -f -- -m ./--

# errors do not stop the other operands
"$OURS/touch" -d 2001-01-01T00:00:00Z h
"$OURS/touch" nodir/x h f2 2> e
eq "touch: an error for one operand, the others are done, status 1" "1 1 1 1" "$? $(wc -l < e | tr -d ' ') $([ "$(mt h)" -gt 1286668800 ] && echo 1) $([ -f f2 ] && echo 1)"
"$OURS/touch" plain_file_x/y 2> /dev/null
rm -f f2

# links and directories
echo t > target
ln -s target lnk
"$OURS/touch" -d 2001-01-01T00:00:00Z target
"$OURS/touch" lnk
eq "touch of a symbolic link touches the target" 1 "$([ "$(mt target)" -gt 1286668800 ] && echo 1)"
ln -s nowhere dang
"$OURS/touch" dang
eq "touch of a dangling link creates its target" "file" "$([ -f nowhere ] && echo file)"
rm -f nowhere
"$OURS/touch" -c dang
eq "touch -c of a dangling link creates nothing" "" "$([ -e nowhere ] && echo made)"
mkdir dd
"$OURS/touch" -d 2001-01-01T00:00:00Z dd
eq "touch of a directory" 978307200 "$(mt dd)"
rm -rf dd lnk dang target

# descriptors
"$OURS/touch" cl >&-
eq "touch with stdout closed: status 0" "0 file" "$? $([ -f cl ] && echo file)"
"$OURS/touch" nodir/x 2>&-
eq "touch with stderr closed, an error: status 1" 1 $?
"$OURS/touch" cl < /dev/null <&-
eq "touch with stdin closed" 0 $?
rm -f cl

#--------------------------------------------------------------------- date
fresh
eq "date: the default format has the shape of the POSIX one" 1 "$("$OURS/date" | grep -Ec '^[A-Z][a-z]{2} [A-Z][a-z]{2} [ 0-3][0-9] [0-2][0-9]:[0-5][0-9]:[0-6][0-9] [A-Za-z0-9+-]* [0-9]{4}$')"
eq "date: the default output is the one of +%a %b %e %H:%M:%S %Z %Y" "$("$OURS/date" '+%a %b %e %H:%M' )" "$("$OURS/date" | sed 's/:[0-9][0-9] .* [0-9]*$//; s/ [0-9]\{4\}$//' | cut -c1-16)"
eq "date: the output ends with a newline and has one line" 1 "$("$OURS/date" | wc -l | tr -d ' ')"
eq "date +%n gives two newlines" "2" "$("$OURS/date" +%n | wc -l | tr -d ' ')"
eq "date + (empty format) gives an empty line" "1 0" "$("$OURS/date" + | wc -l | tr -d ' ') $("$OURS/date" + | tr -d '\n' | wc -c | tr -d ' ')"
eq "date +%% gives a percent sign" "%" "$("$OURS/date" +%%)"
eq "date: text between conversions is copied" "[x]" "$("$OURS/date" '+[x]')"

# the time zone: TZ, and -u as if TZ were UTC0
eq "TZ=JST-9 date +%Z" JST "$(TZ=JST-9 "$OURS/date" +%Z)"
eq "TZ=EST5EDT date +%Z is EST or EDT" 1 "$(TZ=EST5EDT "$OURS/date" +%Z | grep -Ec '^E[SD]T$')"
eq "TZ=EST5EDT date -u +%Z is UTC" UTC "$(TZ=EST5EDT "$OURS/date" -u +%Z)"
eq "date -u +%Z with no TZ" UTC "$(env -u TZ "$OURS/date" -u +%Z)"
eq "date -u ignores TZ=JST-9 for the hour" "$("$OURS/date" -u +%Y%j%H)" "$(TZ=JST-9 "$OURS/date" -u +%Y%j%H)"
eq "TZ=UTC0 date and date -u agree" "$(TZ=UTC0 "$OURS/date" +%Y%j%H)" "$("$OURS/date" -u +%Y%j%H)"
eq "TZ=JST-9 is nine hours ahead of -u" 9 "$(a=$(TZ=JST-9 "$OURS/date" +%H); b=$("$OURS/date" -u +%H); echo $(( (10#$a - 10#$b + 24) % 24 )))"
eq "date -u in the default format ends in UTC and the year" 1 "$("$OURS/date" -u | grep -Ec ' UTC [0-9]{4}$')"
eq "a null TZ is the same as an unset one" "$(env -u TZ "$OURS/date" +%Z)" "$(TZ= "$OURS/date" +%Z)"

# the operand: one at most; a format starts with +; options before it, -- ends them
"$OURS/date" +%Y +%m 2> /dev/null
eq "date with two operands: usage error" 2 $?
"$OURS/date" -x 2> /dev/null
eq "date -x: usage error" 2 $?
"$OURS/date" nonsense 2> e
eq "date nonsense: usage error" "2 usage" "$? $(sed 's/:.*//;q' e)"
eq "date -- +%Y" "$("$OURS/date" +%Y)" "$("$OURS/date" -- +%Y)"
eq "date -u -- +%Y" "$("$OURS/date" -u +%Y)" "$("$OURS/date" -u -- +%Y)"
"$OURS/date" '+%Q' > o 2> e
eq "date +%Q: a diagnostic naming it, nothing on stdout, status 1" "1 0 1" "$? $(wc -c < o | tr -d ' ') $(grep -c '%Q' e)"
"$OURS/date" '+text%' > o 2> e
eq "date: a format ending in % is refused whole" "1 0" "$? $(wc -c < o | tr -d ' ')"

# bytes that are not text
B=$(printf '\377\376')
eq "date keeps raw bytes in the format" "fffe0a" "$("$OURS/date" "+$B" | od -An -tx1 | tr -d ' \n')"
RB=$(printf '+\377%%Y\376')
eq "date keeps raw bytes around a conversion" "ff$(date +%Y | tr -d '\n' | od -An -tx1 | tr -d ' \n')fe0a" "$("$OURS/date" "$RB" | od -An -tx1 | tr -d ' \n')"
big=$(head -c 100000 /dev/zero | tr '\0' 'a')
eq "date with a 100000 byte format" 100001 "$("$OURS/date" "+$big" | wc -c | tr -d ' ')"
eq "date with 20000 conversions" "$(( 20000 * 4 + 1 ))" "$("$OURS/date" "+$(i=0; while [ $i -lt 2000 ]; do printf '%%Y%%Y%%Y%%Y%%Y%%Y%%Y%%Y%%Y%%Y'; i=$((i+1)); done)" | wc -c | tr -d ' ')"

# the set form must not set anything here: invalid forms are refused, and without privilege it fails
before=$("$OURS/date" +%Y%j%H)
"$OURS/date" 13011200 2> e
eq "date 13011200 (month 13): status 2, a diagnostic, clock untouched" "2 1 $before" "$? $(grep -c 'invalid' e) $("$OURS/date" +%Y%j%H)"
"$OURS/date" 0229120023 2> /dev/null
eq "date 0229120023 (no Feb 29 in 2023): status 2" 2 $?
"$OURS/date" 1112101a 2> /dev/null
eq "date with a letter in the digits: usage error" 2 $?
if [ "$(id -u)" != 0 ]; then
	"$OURS/date" 0101000070 2> e
	eq "date 0101000070 without privilege: status 1, cannot set date, clock untouched" "1 1 $before" "$? $(grep -c 'cannot set date' e) $("$OURS/date" +%Y%j%H)"
	"$OURS/date" -u 0101000070 2> e
	eq "date -u 0101000070 without privilege: status 1" 1 $?
else
	echo "skip: date set form (running as root: it would set the clock)"
fi

# descriptors
"$OURS/date" +%Y >&- 2> /dev/null
eq "date with stdout closed: status 1" 1 $?
"$OURS/date" +%Y < /dev/null <&- > /dev/null
eq "date with stdin closed: fine" 0 $?
eq "date does not read its standard input" "left" "$(echo left | { "$OURS/date" > /dev/null; cat; })"
"$OURS/date" +%Y 2>&- > /dev/null
eq "date with stderr closed: fine" 0 $?

# argc == 0: execve() with a null argv (legal for the caller, the kernel allows it).
# No utility may index args[0] or args[1..] blindly and abort.
if command -v cc > /dev/null 2>&1; then
	cat > nullargv.c <<'CEOF'
#include <unistd.h>
extern char **environ;
int main(int argc, char **argv) { (void)argc; execve(argv[1], 0, environ); return 127; }
CEOF
	if cc -o nullargv nullargv.c 2> /dev/null; then
		crashed=
		for b in "$OURS"/*; do
			[ -f "$b" ] && [ -x "$b" ] || continue
			case $b in *.d | *.rlib) continue ;; esac
			./nullargv "$b" < /dev/null > /dev/null 2>&1
			case $? in 132 | 133 | 134 | 139) crashed="$crashed ${b##*/}" ;; esac
		done
		eq "no utility crashes when started with a null argv" "" "$crashed"
	else
		echo "skip: no C compiler that works"
	fi
else
	echo "skip: no C compiler"
fi

echo "regress: $((pass + fail)) checks, $fail failed"
[ "$fail" -eq 0 ]
