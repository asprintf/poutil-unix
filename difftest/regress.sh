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
# st_mode, st_mtime_s, ...: the same text from BSD stat (macOS) and GNU stat (note that GNU "stat -f" is a different thing)
. "$here/lib/stat.sh"

# a UTF-8 locale this system has: en_US.UTF-8 on macOS and most Linux systems, C.UTF-8 or C.utf8 on a bare one
U8=en_US.UTF-8
for l in en_US.UTF-8 en_US.utf8 C.UTF-8 C.utf8; do
	if [ "$(LC_ALL=$l locale charmap 2> /dev/null)" = UTF-8 ]; then
		U8=$l
		break
	fi
done

# md5: macOS has md5, Linux md5sum
echo x | md5 > /dev/null 2>&1 || md5() { md5sum | cut -d ' ' -f 1; }

# deeptree DIR N FILES: N nested directories below DIR, a file in each if FILES is 1. This is deeper than
# PATH_MAX, so the shell has to be able to cd that deep: dash builds the whole path and cannot, bash can.
deeptree() {
	deepsh=sh
	command -v bash > /dev/null 2>&1 && deepsh=bash
	$deepsh -c 'cd "$1" || exit 1; i=0; while [ $i -lt $2 ]; do mkdir dddddddddddddddd || exit 1; cd dddddddddddddddd || exit 1; [ "$3" = 1 ] && : > file; i=$((i + 1)); done' deeptree "$1" "$2" "$3"
}
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
big=$(head -c 100000 /dev/zero | tr '\0' x)
eq "echo: a 100000-byte operand" 100001 "$("$OURS/echo" "$big" | wc -c | tr -d ' ')"
eq "echo: 20000 operands" "$(seq 1 20000 | tr '\n' ' ' | sed 's/ $//')" "$("$OURS/echo" $(seq 1 20000))"
"$OURS/echo" hi <&- > /dev/null
eq "echo with stdin closed: status" 0 $?
"$OURS/echo" hi >&- 2> err
eq "echo with stdout closed: status" 1 $?
eq "echo with stdout closed: one diagnostic" 1 "$(wc -l < err | tr -d ' ')"
{ "$OURS/echo" $(seq 1 50000); echo $? > st; } | head -c 10 > /dev/null
eq "echo into a closed pipe dies of SIGPIPE" 141 "$(cat st)"
if [ "$(LC_ALL=$U8 locale charmap 2> /dev/null)" = UTF-8 ]; then
	E=$(printf '\303\251')
	eq "echo: a multibyte character passes through" "c3a90a" "$(LC_ALL=$U8 "$OURS/echo" "$E" | hex)"
	eq "echo: an escape next to a multibyte character" "c3a9090a" "$(LC_ALL=$U8 "$OURS/echo" "$E\\t" | hex)"
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

#------------------------------------------------- xargs -p on a terminal
# -p reads its answers from /dev/tty and writes "?..." at the end of the line of the -t output. script(1) gives
# xargs a terminal; its standard input is typed on it a second later, when the echo is off, so that the screen shows
# what xargs wrote.
fresh
if [ -x /usr/bin/script ] && /usr/bin/script -q /dev/null true < /dev/null > /dev/null 2>&1; then
	ptyrun() { (sleep 1; printf '%s' "$1") | /usr/bin/script -q /dev/null sh -c "stty -echo; $2" > ty 2>&1; tr -d '\r' < ty; }
elif [ -x /usr/bin/script ] && /usr/bin/script -qec true /dev/null < /dev/null > /dev/null 2>&1; then
	ptyrun() { (sleep 1; printf '%s' "$1") | /usr/bin/script -qec "stty -echo; $2" /dev/null 2>&1 | tr -d '\r'; }
else
	ptyrun() { return 1; }
fi
if ptyrun '' true > /dev/null 2>&1; then
	export OURS
	eq "xargs -p: yes runs the utility; the prompt follows the -t line" "echo hi a b?...hi a b" "$(ptyrun 'y
' 'echo "a b" | "$OURS/xargs" -p echo hi')"
	eq "xargs -p: no skips it" "echo hi a b?..." "$(ptyrun 'n
' 'echo "a b" | "$OURS/xargs" -p echo hi')"
	eq "xargs -p: anything that does not start with y is no" "echo hi a?..." "$(ptyrun 'maybe
' 'echo a | "$OURS/xargs" -p echo hi')"
	eq "xargs -p: Yes (an answer that starts with y or Y) is yes" "echo hi a?...hi a" "$(ptyrun 'Yes
' 'echo a | "$OURS/xargs" -p echo hi')"
	eq "xargs -p -n 1: each invocation is asked about (no answer is echoed, so a no leaves the next line on the same row)" "echo hi a?...hi a|echo hi b?...echo hi c?...hi c" "$(ptyrun 'y
n
y
' 'echo a b c | "$OURS/xargs" -p -n 1 echo hi' | tr '\n' '|' | sed 's/|$//')"
	eq "xargs -p: the arguments are shown apart" "echo 'a b'?..." "$(ptyrun 'n
' 'echo "\"a b\"" | "$OURS/xargs" -p echo')"
	eq "xargs -p: the status is 0 when the answers were no" "0" "$(ptyrun 'n
' 'echo a | "$OURS/xargs" -p echo hi; echo $?' | tail -1 | sed 's/.*?\.\.\.//')"
else
	echo "skip: no script(1) with a usable terminal for xargs -p"
fi

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
	ttyrun '"$OURS/tty" > "$W/ours"; echo $? > "$W/st"; /usr/bin/tty > "$W/sys"'
	eq "tty on a terminal: status 0, the name the system's tty gives" "0 $(cat "$W/sys")" "$(cat "$W/st") $(cat "$W/ours")"
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
"$OURS/tty" < /dev/null >&- 2> /dev/null
eq "tty with stdout closed: status 2, not 1 (not a terminal)" 2 $?
(trap '' PIPE; { sleep 1; "$OURS/tty" < /dev/null 2> err; echo $? > st; } | true)
eq "tty into a broken pipe, SIGPIPE ignored: status 2 and a diagnostic" "2 1" "$(cat st) $(wc -l < err | tr -d ' ')"

#---------------------------------------------------------------------- uname
# the system's own figures, from sysctl, are the reference
fresh
s=$(sysctl -n kern.ostype 2> /dev/null)
if [ -n "$s" ]; then
	n=$(sysctl -n kern.hostname)
	r=$(sysctl -n kern.osrelease)
	v=$(sysctl -n kern.version)
	m=$(sysctl -n hw.machine)
else
	# no BSD sysctl (Linux): the system's own uname is the reference
	s=$(/usr/bin/uname -s)
	n=$(/usr/bin/uname -n)
	r=$(/usr/bin/uname -r)
	v=$(/usr/bin/uname -v)
	m=$(/usr/bin/uname -m)
fi
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
if [ "$(LC_ALL=$U8 locale charmap 2> /dev/null)" = UTF-8 ]; then
	E=$(printf '\303\251')
	# 100000 lines of "e-acute newline": 64 KB blocks cut some of the characters in two
	eq "wc -mw counts characters whole over a pipe" "100000 200000" \
		"$(yes "$E" | head -n 100000 | LC_ALL=$U8 "$OURS/wc" -wm)"
	eq "wc -c over the same pipe counts bytes" 300000 \
		"$(yes "$E" | head -n 100000 | LC_ALL=$U8 "$OURS/wc" -c)"
	eq "wc -m in the C locale counts bytes" 300000 \
		"$(yes "$E" | head -n 100000 | LC_ALL=C "$OURS/wc" -m)"
else
	echo "skip: no $U8 locale"
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
{ head -c 1000000 /dev/zero | tr '\0' y; echo " z"; } > longitem
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
"$T" -t x 2> err
eq "test -t x: false, as the spec says for a bad descriptor number: status" 1 $?
eq "test -t x: no diagnostic" 0 "$(wc -c < err | tr -d ' ')"
"$T" -t -1 2> err
eq "test -t -1: false: status" 1 $?
eq "test -t -1: no diagnostic" 0 "$(wc -c < err | tr -d ' ')"
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
eq "expr with stdout closed: status 3 (1 and 2 mean something else)" 3 $?
(trap '' PIPE; { sleep 1; "$OURS/expr" 1 + 1 2> err; echo $? > st; } | true)
eq "expr into a broken pipe, SIGPIPE ignored: status 3 and a diagnostic" "3 1" "$(cat st) $(wc -l < err | tr -d ' ')"
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
LC_ALL=$U8
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
	echo "skip: no $U8 locale"
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

#---------------------------------------------------------------------- join
fresh
printf '1 a\n2 b\n4 d\n' > f1
printf '2 x\n3 y\n4 z\n' > f2
"$OURS/join" f1 f2 > out
eq "join: status and the paired lines" "0 2 b x/4 d z/" "$? $(tr '\n' '/' < out)"
"$OURS/join" -a 1 -a 2 f1 f2 > out
eq "join -a 1 -a 2: all unpairable lines too" "0 1 a/2 b x/3 y/4 d z/" "$? $(tr '\n' '/' < out)"
"$OURS/join" -v 1 -v 2 f1 f2 > out
eq "join -v 1 -v 2: only the unpairable lines" "0 1 a/3 y/" "$? $(tr '\n' '/' < out)"
"$OURS/join" -o 0,1.2,2.2 -e - -a 2 f1 f2 > out
eq "join -o -e: a missing field is the -e string" "0 2 b x/3 - y/4 d z/" "$? $(tr '\n' '/' < out)"
printf 'Name\tPhone Number\nDon\t+1 123-456-7890\nHal\t+1 234-567-8901\nYasushi\t+2 345-678-9012\n' > phone
printf 'Name\tFax Number\nDon\t+1 123-456-7899\nKeith\t+1 456-789-0122\nYasushi\t+2 345-678-9011\n' > fax
"$OURS/join" -t '	' -a 1 -a 2 -e '(unknown)' -o 0,1.2,2.2 phone fax > out
eq "join: the EXAMPLES phone and fax" "0 Name>Phone Number>Fax Number/Don>+1 123-456-7890>+1 123-456-7899/Hal>+1 234-567-8901>(unknown)/Keith>(unknown)>+1 456-789-0122/Yasushi>+2 345-678-9012>+2 345-678-9011/" "$? $(tr '\t\n' '>/' < out)"
printf 'a x\na y\na z\n' > fa
printf 'a p\n' > fb
"$OURS/join" fa fb > out
eq "join: the EXAMPLES one key three times" "a x p/a y p/a z p/" "$(tr '\n' '/' < out)"
printf 'a b c\na d e\n' > ga
printf 'a w x\na y z\na o p\n' > gb
"$OURS/join" ga gb > out
eq "join: the EXAMPLES every combination, file 1 outer" "a b c w x/a b c y z/a b c o p/a d e w x/a d e y z/a d e o p/" "$(tr '\n' '/' < out)"
printf '\n\n1 a\n2 b' > g1
printf '\n2 c\n' > g2
"$OURS/join" -a 1 g1 g2 > out
eq "join: blank lines are lines, a last line needs no newline" " / /1 a/2 b c/" "$(sed 's/^$/ /' out | tr '\n' '/')"
"$OURS/join" f1 f2 f1 2> /dev/null
eq "join with three operands is a usage error" 2 $?
"$OURS/join" f1 2> /dev/null
eq "join with one operand is a usage error" 2 $?
"$OURS/join" 2> /dev/null
eq "join without operands is a usage error" 2 $?
"$OURS/join" -a 3 f1 f2 2> /dev/null
eq "join -a 3 is a usage error" 2 $?
"$OURS/join" -v 0 f1 f2 2> /dev/null
eq "join -v 0 is a usage error" 2 $?
"$OURS/join" -a 1 -v 2 f1 f2 2> /dev/null
eq "join -a with -v is a usage error" 2 $?
"$OURS/join" -1 0 f1 f2 2> /dev/null
eq "join -1 0 is a usage error" 2 $?
"$OURS/join" -o 3.1 f1 f2 2> /dev/null
eq "join -o 3.1 is a usage error" 2 $?
"$OURS/join" -t ab f1 f2 2> /dev/null
eq "join -t of two characters is a usage error" 2 $?
"$OURS/join" -j 1 f1 f2 2> /dev/null
eq "join -j (removed in Issue 6) is a usage error" 2 $?
"$OURS/join" -a 2> /dev/null
eq "join -a without its argument is a usage error" 2 $?
"$OURS/join" f1 nosuch > out 2> err
eq "join of a missing file: status, no output, a diagnostic" "1 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
"$OURS/join" nosuch f1 2> /dev/null
eq "join of a missing first file: status" 1 $?
"$OURS/join" f1 . > out 2> err
eq "join of a directory: status, no output, a diagnostic" "1 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
"$OURS/join" f1 -t > out 2> err
eq "join f1 -t: the -t after an operand is a file name" "1 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
"$OURS/join" f1 -- > out 2> err
eq "join f1 --: the -- after an operand is a file name" "1 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
"$OURS/join" -- f1 f2 > out
eq "join -- f1 f2: the -- ends the options" "0 2 b x/4 d z/" "$? $(tr '\n' '/' < out)"
printf '1 a\n2 b\n4 d\n' | "$OURS/join" - f2 > out
eq "join - reads standard input as file1" "2 b x/4 d z/" "$(tr '\n' '/' < out)"
printf '2 x\n3 y\n4 z\n' | "$OURS/join" f1 - > out
eq "join - reads standard input as file2" "2 b x/4 d z/" "$(tr '\n' '/' < out)"
: > empty
"$OURS/join" empty empty > out
eq "join of two empty files: status, no output" "0 0" "$? $(wc -c < out | tr -d ' ')"
"$OURS/join" f1 f2 >&- 2> /dev/null
eq "join with stdout closed: status" 1 $?
"$OURS/join" - f1 <&- 2> /dev/null
eq "join with stdin closed: status" 1 $?
awk 'BEGIN { for (i = 0; i < 200000; i++) printf "%07d a\n", i }' > big1
awk 'BEGIN { for (i = 0; i < 200000; i++) printf "%07d b\n", i + 100000 }' > big2
"$OURS/join" big1 big2 > out
eq "join of two big files: the common 100000 lines" "100000 0100000 a b" "$(wc -l < out | tr -d ' ') $(head -n 1 out)"
"$OURS/join" -a 1 -a 2 big1 big2 | head -n 2 > out
eq "join into a closed pipe ends quietly" "0000000 a/0000001 a/" "$(tr '\n' '/' < out)"
printf 'k a\n' > h1
awk 'BEGIN { for (i = 0; i < 3000; i++) printf "k b%d\n", i }' > h2
"$OURS/join" h1 h2 | wc -l | tr -d ' ' > out
eq "join: one line against 3000 lines with the same key" 3000 "$(cat out)"

#---------------------------------------------------------------------- cal
fresh
printf '   February 2024\nSu Mo Tu We Th Fr Sa\n             1  2  3\n 4  5  6  7  8  9 10\n11 12 13 14 15 16 17\n18 19 20 21 22 23 24\n25 26 27 28 29\n' > want
"$OURS/cal" 2 2024 > out
eq "cal 2 2024: status" 0 $?
eq "cal 2 2024: the month" "$(cat want)" "$(cat out)"
printf '   September 1752\nSu Mo Tu We Th Fr Sa\n       1  2 14 15 16\n17 18 19 20 21 22 23\n24 25 26 27 28 29 30\n' > want
"$OURS/cal" 9 1752 > out
eq "cal 9 1752: the days 3 to 13 do not exist" "$(cat want)" "$(cat out)"
"$OURS/cal" 2026 > out
eq "cal 2026: twelve months and no blank at the end of a line" "12 0" "$(grep -o 'Su Mo Tu We Th Fr Sa' out | wc -l | tr -d ' ') $(grep -c '[[:space:]]$' out | tr -d ' ')"
eq "cal 2026: the year is the first line" 2026 "$(head -n 1 out | tr -d ' ')"
"$OURS/cal" 83 > out
eq "cal 83 is the year 83, not 1983" "83 12" "$(head -n 1 out | tr -d ' ') $(grep -o 'Su Mo Tu We Th Fr Sa' out | wc -l | tr -d ' ')"
"$OURS/cal" -- 3 2026 > out
eq "cal -- 3 2026: the -- ends the options" "March 2026" "$(head -n 1 out | sed 's/^ *//')"
"$OURS/cal" > out
eq "cal: the current month, as date says" "$(date '+%B %Y')" "$(head -n 1 out | sed 's/^ *//')"
TZ=Etc/GMT+12 "$OURS/cal" > out
eq "cal follows TZ (UTC-12)" "$(TZ=Etc/GMT+12 date '+%B %Y')" "$(head -n 1 out | sed 's/^ *//')"
TZ=Pacific/Kiritimati "$OURS/cal" > out
eq "cal follows TZ (UTC+14)" "$(TZ=Pacific/Kiritimati date '+%B %Y')" "$(head -n 1 out | sed 's/^ *//')"
TZ= "$OURS/cal" > out
eq "cal with a null TZ: still a month" 1 "$(grep -c 'Su Mo Tu We Th Fr Sa' out | tr -d ' ')"
"$OURS/cal" 1 2 3 2> /dev/null
eq "cal with three operands is a usage error" 2 $?
"$OURS/cal" -y 2026 2> /dev/null
eq "cal -y is a usage error (it has no options)" 2 $?
"$OURS/cal" -1 2> /dev/null
eq "cal -1 is a usage error" 2 $?
for bad in 0 10000 abc '13 2020' '0 2020' '1 0' '2020 1' 1.5 +5 99999999999999999999; do
	"$OURS/cal" $bad > out 2> err
	eq "cal $bad: status 1, no output, a diagnostic" "1 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
done
"$OURS/cal" 2026 >&- 2> /dev/null
eq "cal with stdout closed: status" 1 $?
"$OURS/cal" 2026 <&- > /dev/null
eq "cal with stdin closed: fine, it reads nothing" 0 $?
"$OURS/cal" 2026 | head -n 1 > out
eq "cal into a closed pipe ends quietly" 2026 "$(tr -d ' ' < out)"
fr=$(LC_ALL=fr_FR.UTF-8 "$OURS/cal" 3 2020 | sed -n 1p | sed 's/^ *//')
if [ "$fr" = "March 2020" ]; then
	echo "skip: no locale fr_FR.UTF-8"
else
	eq "cal in fr_FR.UTF-8: LC_TIME gives the names" "mars 2020/di lu ma me je ve sa" "$(LC_ALL=fr_FR.UTF-8 "$OURS/cal" 3 2020 | sed -n 1,2p | sed 's/^ *//' | tr '\n' '/' | sed 's/\/$//')"
	LC_ALL= LC_TIME= LANG=fr_FR.UTF-8 "$OURS/cal" 3 2020 > out
	eq "cal: LANG is the default of LC_TIME" "mars 2020" "$(head -n 1 out | sed 's/^ *//')"
	LC_ALL=C LC_TIME=fr_FR.UTF-8 "$OURS/cal" 3 2020 > out
	eq "cal: LC_ALL overrides LC_TIME" "March 2020" "$(head -n 1 out | sed 's/^ *//')"
fi

#---------------------------------------------------------------------- df
fresh
mkdir sub
mkfifo pipe
: > file
"$OURS/df" -P / > out 2> err
eq "df -P /: status, a header and one line, silence" "0 2 0" "$? $(wc -l < out | tr -d ' ') $(wc -c < err | tr -d ' ')"
eq "df -P /: the header of the standard" "Filesystem 512-blocks Used Available Capacity Mounted on" "$(sed -n 1p out)"
eq "df -P /: the line is exactly \"%s %d %d %d %d%% %s\"" 1 "$(sed -n 2p out | grep -c -E '^[^ ]+ [0-9]+ [0-9]+ -?[0-9]+ [0-9]+% /$' | tr -d ' ')"
"$OURS/df" -kP / > outk
eq "df -kP /: the header says 1024-blocks" "Filesystem 1024-blocks Used Available Capacity Mounted on" "$(sed -n 1p outk)"
eq "df -kP /: the total is half of the 512-byte one, rounded up" 1 "$(awk 'NR == FNR { if (FNR == 2) t = $2; next } FNR == 2 { print ($2 * 2 - t == 0 || $2 * 2 - t == 1) ? 1 : 0 }' out outk)"
"$OURS/df" -P / > a
POSIXLY_CORRECT=1 /bin/df -P / > b
eq "df -P / agrees with the system df (total exactly, used and available to 50 MB, capacity to 1)" 1 "$(awk 'NR == FNR { if (FNR == 2) { t = $2; u = $3; v = $4; c = $5 + 0 } next } FNR == 2 { d1 = $3 - u; d2 = $4 - v; if (d1 < 0) d1 = -d1; if (d2 < 0) d2 = -d2; dc = ($5 + 0) - c; if (dc < 0) dc = -dc; print ($2 == t && d1 < 100000 && d2 < 100000 && dc <= 1) ? 1 : 0 }' a b)"
eq "df -P /: the capacity is used/(used+available) rounded up" 1 "$(awk 'NR == 2 { p = $3 * 100 / ($3 + $4); c = (p == int(p)) ? p : int(p) + 1; sub("%", "", $5); print (c == $5) ? 1 : 0 }' a)"
eq "df -P /: used and available do not pass the total" 1 "$(awk 'NR == 2 { print ($3 + $4 <= $2) ? 1 : 0 }' a)"
"$OURS/df" / > out
eq "df /: the header is the same without -P" "Filesystem 512-blocks Used Available Capacity Mounted on" "$(sed -n 1p out)"
"$OURS/df" -t / > out
eq "df -t /: and with -t" "Filesystem 512-blocks Used Available Capacity Mounted on" "$(sed -n 1p out)"
"$OURS/df" -P . > a
"$OURS/df" -P sub > b
eq "df -P dir and df -P dir/sub name the same file system (EXAMPLES)" "$(sed -n 2p a | awk '{ print $1, $NF }')" "$(sed -n 2p b | awk '{ print $1, $NF }')"
"$OURS/df" -P file pipe sub > out 2> err
eq "df of a regular file, a FIFO and a directory: three lines after one header" "0 4 0" "$? $(wc -l < out | tr -d ' ') $(wc -c < err | tr -d ' ')"
"$OURS/df" -P / /dev | awk 'NR > 1 { print $NF }' > out
eq "df -P / /dev: the operands in the order given" "/ /dev" "$(tr '\n' ' ' < out | sed 's/ $//')"
"$OURS/df" -P /dev / | awk 'NR > 1 { print $NF }' > out
eq "df -P /dev /: and the other way round" "/dev /" "$(tr '\n' ' ' < out | sed 's/ $//')"
"$OURS/df" -P /nosuch / > out 2> err
eq "df -P /nosuch /: it goes on, the status says it failed" "1 2 1" "$? $(wc -l < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
"$OURS/df" -P /nosuch > out 2> err
eq "df -P /nosuch: status 1, no output, a diagnostic that names it" "1 0 1" "$? $(wc -c < out | tr -d ' ') $(grep -c '/nosuch' err | tr -d ' ')"
if ln -s '' empty 2> /dev/null; then
	"$OURS/df" -P empty > out 2> err
	s=$?
	[ "$s" -ne 0 ] && s=nonzero
	eq "df of a symbolic link with an empty target: a diagnostic and a non-zero status" "nonzero 1" "$s $(wc -l < err | tr -d ' ')"
else
	echo "skip: this file system refuses a symbolic link with an empty target"
fi
ln -s nosuch dangling
"$OURS/df" -P dangling > out 2> err
eq "df of a dangling link: a diagnostic and status 1" "1 1" "$? $(wc -l < err | tr -d ' ')"
"$OURS/df" -P /dev/null > out 2> err
eq "df of a special file that is no mounted device: status 1, a diagnostic" "1 1 not mounted" "$? $(wc -l < err | tr -d ' ') $(sed 's/.*: //' err)"
"$OURS/df" -P / > a
dev=$(sed -n 2p a | awk '{ print $1 }')
if [ -b "$dev" ]; then
	"$OURS/df" -P "$dev" > out 2> err
	eq "df of the device of a mounted file system: that file system" "0 /" "$? $(sed -n 2p out | awk '{ print $NF }')"
else
	echo "skip: the file system of / has no device node here ($dev)"
fi
"$OURS/df" -P > out 2> err
eq "df -P: every file system, one header, status 0, silence" "0 1 0" "$? $(grep -c '^Filesystem' out | tr -d ' ') $(wc -c < err | tr -d ' ')"
eq "df -P: the root is among them, all lines have at least six fields" "1 0" "$(awk 'NR > 1 && $NF == "/"' out | wc -l | tr -d ' ') $(awk 'NR > 1 && NF < 6' out | wc -l | tr -d ' ')"
"$OURS/df" -P -- / > out
eq "df -P -- /: the -- ends the options" "0 2" "$? $(wc -l < out | tr -d ' ')"
"$OURS/df" -P / -k > out 2> err
eq "df -P / -k: the -k after an operand is a file name" "1 2 1" "$? $(wc -l < out | tr -d ' ') $(grep -c -- '-k' err | tr -d ' ')"
"$OURS/df" -P -t / 2> /dev/null
eq "df -P -t is a usage error (the synopsis: -P or -t)" 2 $?
"$OURS/df" -x / 2> /dev/null
eq "df -x is a usage error" 2 $?
"$OURS/df" -P / >&- 2> /dev/null
eq "df with stdout closed: status" 1 $?
"$OURS/df" -P / <&- > /dev/null
eq "df with stdin closed: fine, it reads nothing" 0 $?
"$OURS/df" -P | head -n 1 > out
eq "df into a closed pipe ends quietly" "Filesystem 512-blocks Used Available Capacity Mounted on" "$(cat out)"

#---------------------------------------------------------------------- du
fresh
mkdir -p t/a/b t/c t/empty
printf x > t/f1
head -c 5000 /dev/zero > t/a/f2
printf y > t/a/b/f3
head -c 70000 /dev/zero > t/c/big
ln -s a t/lnd
ln -s f1 t/lnf
ln t/f1 t/c/hard
"$OURS/du" t/f1 > out 2> err
eq "du of a file: status, one line, silence" "0 1 0" "$? $(wc -l < out | tr -d ' ') $(wc -c < err | tr -d ' ')"
eq "du of a file: exactly \"%d %s\" with one blank" 1 "$(grep -c -E '^[0-9]+ t/f1$' out | tr -d ' ')"
"$OURS/du" -a t/a | sed 's/^\([0-9][0-9]*\)[[:space:]]/\1 /' | sort > a
POSIXLY_CORRECT=1 /usr/bin/du -a t/a | sed 's/^\([0-9][0-9]*\)[[:space:]]/\1 /' | sort > b
eq "du -a t/a: the same sizes and names as the system du" "" "$(diff a b)"
"$OURS/du" -ak t/a | sed 's/^\([0-9][0-9]*\)[[:space:]]/\1 /' | sort > a
/usr/bin/du -ak t/a | sed 's/^\([0-9][0-9]*\)[[:space:]]/\1 /' | sort > b
eq "du -ak t/a: and in 1024-byte units" "" "$(diff a b)"
"$OURS/du" -s t > out
eq "du -s t: the total of the operand, as the system du has it" "$(POSIXLY_CORRECT=1 /usr/bin/du -s t | awk '{ print $1 }') t" "$(awk '{ print $1, $2 }' out)"
"$OURS/du" -sk t > outk
"$OURS/du" -s t > out
eq "du -sk is the total of du -s in 1024-byte units, rounded up" 1 "$(awk 'NR == FNR { b = $1; next } { print ($1 == int((b + 1) / 2)) ? 1 : 0 }' out outk)"
"$OURS/du" t/a t/c > out
eq "du: a directory is written after the directories below it" "t/a/b t/a t/c" "$(awk '{ print $2 }' out | tr '\n' ' ' | sed 's/ $//')"
"$OURS/du" t/c t/a > out
eq "du: the operands in the order given" "t/c t/a/b t/a" "$(awk '{ print $2 }' out | tr '\n' ' ' | sed 's/ $//')"
"$OURS/du" -a t > out
eq "du -a t: t/f1 and t/c/hard are one entry" 1 "$(grep -c -E 't/f1$|t/c/hard$' out | tr -d ' ')"
"$OURS/du" t/lnd > out
eq "du of a link: the link itself" "t/lnd" "$(awk '{ print $2 }' out | tr '\n' ' ' | sed 's/ $//')"
"$OURS/du" -H t/lnd > out
eq "du -H of a link to a directory: its contents, under the link's name" "t/lnd/b t/lnd" "$(awk '{ print $2 }' out | tr '\n' ' ' | sed 's/ $//')"
"$OURS/du" -L -H t/lnd > out
eq "du -L -H: the last one decides" "t/lnd/b t/lnd" "$(awk '{ print $2 }' out | tr '\n' ' ' | sed 's/ $//')"
"$OURS/du" -H -L t/lnd > out
eq "du -H -L: the last one decides" "t/lnd/b t/lnd" "$(awk '{ print $2 }' out | tr '\n' ' ' | sed 's/ $//')"
mkdir cyc
ln -s .. cyc/up
"$OURS/du" -L cyc > out 2> err
eq "du -L of a link back to a parent: an error, not endless" "1 1" "$? $(grep -c cycle err | tr -d ' ')"
"$OURS/du" nosuch t/f1 nosuch2 > out 2> err
eq "du of a missing file: it goes on, status 1, one diagnostic each" "1 1 2" "$? $(wc -l < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
ln -s nosuch dang
"$OURS/du" dang > out 2> err
eq "du of a dangling link: the link, no error" "0 1 0" "$? $(wc -l < out | tr -d ' ') $(wc -c < err | tr -d ' ')"
"$OURS/du" -H dang > out 2> err
eq "du -H of a dangling link: an error" "1 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
ln -s lp2 lp1
ln -s lp1 lp2
"$OURS/du" lp1/x > out 2> err
eq "du of a path through a loop of links: an error" "1 1" "$? $(wc -l < err | tr -d ' ')"
if [ "$(id -u)" -ne 0 ]; then
	mkdir -p lock/open lock/shut
	: > lock/shut/f
	chmod 000 lock/shut
	"$OURS/du" lock > out 2> err
	eq "du of an unreadable directory: status 1, a diagnostic, it is still written" "1 1 lock/open lock/shut lock" "$? $(wc -l < err | tr -d ' ') $(awk '{ print $2 }' out | tr '\n' ' ' | sed 's/ $//')"
	chmod 755 lock/shut
fi
mkdir cwd
: > cwd/f
( cd cwd && "$OURS/du" -a > ../out )
eq "du without an operand: the current directory" "./f ." "$(awk '{ print $2 }' out | tr '\n' ' ' | sed 's/ $//')"
"$OURS/du" -a -s t 2> /dev/null
eq "du -a -s is a usage error" 2 $?
"$OURS/du" -b t 2> /dev/null
eq "du -b is a usage error" 2 $?
"$OURS/du" -- t/f1 > out
eq "du -- t/f1: the -- ends the options" "0 1" "$? $(wc -l < out | tr -d ' ')"
"$OURS/du" t/f1 -s > out 2> err
eq "du t/f1 -s: the -s after an operand is a file name" "1 1 1" "$? $(wc -l < out | tr -d ' ') $(wc -l < err | tr -d ' ')"
if [ -d /Volumes/vscrw ] && [ "$(stat -f %d /Volumes/vscrw)" != "$(stat -f %d /Volumes)" ]; then
	"$OURS/du" /Volumes > out 2> /dev/null
	n1=$(grep -c '/Volumes/vscrw' out | tr -d ' ')
	"$OURS/du" -x /Volumes > out 2> /dev/null
	n2=$(grep -c '/Volumes/vscrw' out | tr -d ' ')
	eq "du -x /Volumes leaves out the file systems mounted below it" "yes none" "$([ "$n1" -ge 1 ] && echo yes) $([ "$n2" -eq 0 ] && echo none)"
else
	echo "skip: no other file system mounted below /Volumes"
fi
mkdir deep
deeptree deep 40 1
"$OURS/du" -s deep > out 2> err
eq "du -s of a tree 40 levels deep (over 700 bytes of path)" "0 1" "$? $(wc -l < out | tr -d ' ')"
(ulimit -s 512 2> /dev/null; "$OURS/du" deep > out 2> err; echo $? > st)
eq "du with a small stack, 40 levels deep" "0 41" "$(cat st) $(wc -l < out | tr -d ' ')"
rm -rf deep
mkdir deep
deeptree deep 300 0
"$OURS/du" -s deep > out 2> err
eq "du of a tree deeper than PATH_MAX: a diagnostic and status 1, no crash" "1 1 File name too long" "$? $(wc -l < err | tr -d ' ') $(sed 's/.*: //' err | sort -u)"
rm -rf deep
mkdir wide
( cd wide && i=0 && while [ $i -lt 3000 ]; do : > "file$i"; i=$((i + 1)); done )
"$OURS/du" -a wide > out
eq "du -a of a directory with 3000 entries" 3001 "$(wc -l < out | tr -d ' ')"
rm -rf wide
"$OURS/du" t >&- 2> /dev/null
eq "du with stdout closed: status" 1 $?
"$OURS/du" t <&- > /dev/null
eq "du with stdin closed: fine, it reads nothing" 0 $?
"$OURS/du" -a /usr 2> /dev/null | head -n 1 > out
eq "du into a closed pipe ends quietly" 1 "$(wc -l < out | tr -d ' ')"

#---------------------------------------------------------------------- printf
fresh
"$OURS/printf" 'hello %s\n' world > out 2> err
eq "printf: status, output, silence" "0 hello world 0" "$? $(cat out | tr '\n' ' ' | sed 's/ $//') $(wc -c < err | tr -d ' ')"
"$OURS/printf" '\aPlease fill in the following: \nName: ' | od -An -tx1 | tr -d ' \n' > out
eq "printf: the EXAMPLES prompt, a bell first" "07506c65617365" "$(cut -c1-14 out)"
"$OURS/printf" '%2d right\t%2d wrong\t(%s%%)\n' 3 1 75.0 | tr '\t' '>' > out
eq "printf: the EXAMPLES right and wrong, tabs between" " 3 right> 1 wrong>(75.0%)" "$(cat out)"
"$OURS/printf" '%5d%4d\n' 1 21 321 4321 54321 > out
eq "printf: the EXAMPLES three lines from one format" "    1  21/  3214321/54321   0/" "$(tr '\n' '/' < out)"
"$OURS/printf" '%d\n' 3 +3 -3 \'3 \"+3 "'-3" > out 2> err
eq "printf: the EXAMPLES character constants" "0 3 3 -3 51 43 45 0" "$? $(tr '\n' ' ' < out | sed 's/ $//') $(wc -c < err | tr -d ' ')"
"$OURS/printf" '%d\n' 5a > out 2> err
eq "printf %d 5a: the value so far, a diagnostic, status 1" "1 5 1" "$? $(cat out) $(wc -l < err | tr -d ' ')"
"$OURS/printf" '%d\n' 9999999999999999999999 -9999999999999999999999 ABC > out 2> err
eq "printf: overflow and a non-number: the values of strtoimax, a diagnostic each" "1 9223372036854775807/-9223372036854775808/0/ 3" "$? $(tr '\n' '/' < out) $(wc -l < err | tr -d ' ')"
"$OURS/printf" '[%d][%d][%d]\n' 1 x 3 > out 2> err
eq "printf: an error does not stop the other arguments" "1 [1][0][3] 1" "$? $(cat out) $(wc -l < err | tr -d ' ')"
"$OURS/printf" '%d\n' > out 2> err
eq "printf: a missing argument is a zero, without a message" "0 0 0" "$? $(cat out) $(wc -c < err | tr -d ' ')"
"$OURS/printf" 'x\0y\377z' | od -An -tx1 | tr -d ' \n' > out
eq "printf: octal escapes make any byte" "780079ff7a" "$(cat out)"
"$OURS/printf" '%b' 'a\0101\0b\033[0m' | od -An -tx1 | tr -d ' \n' > out
eq "printf %b: \\0ddd and the other escapes" "614100621b5b306d" "$(cat out)"
"$OURS/printf" '[%b][%s]\n' 'ab\ccd' next > out
eq "printf %b: \\c ends everything, the rest of the format too" "[ab" "$(cat out)"
"$OURS/printf" '%s %s %s\n' a b > out
eq "printf: a missing string is empty" "a b " "$(cat out)"
"$OURS/printf" 'no conversion\n' ignored arguments > out 2> err
eq "printf: a format with no conversion and arguments: once, silently" "0 no conversion 0" "$? $(cat out) $(wc -c < err | tr -d ' ')"
"$OURS/printf" '%s\n' a b c > out
eq "printf: the format is used for each argument" "a/b/c/" "$(tr '\n' '/' < out)"
"$OURS/printf" 'before %z after\n' 1 > out 2> err
eq "printf: an invalid conversion: what came before, a diagnostic, status 1" "1 before  1" "$? $(cat out) $(wc -l < err | tr -d ' ')"
"$OURS/printf" '%f|%.2e|%g|%8.3f|\n' 1.5 1234 100000 3.14159 > out 2> err
eq "printf: the floating-point conversions" "0 1.500000|1.23e+03|100000|   3.142| 0" "$? $(cat out) $(wc -c < err | tr -d ' ')"
"$OURS/printf" '%f\n' abc 1.5x 1e999 > out 2> err
eq "printf %f: wrong arguments: the value so far, a diagnostic each, status 1" "1 0.000000/1.500000/inf/ 3" "$? $(tr '\n' '/' < out) $(wc -l < err | tr -d ' ')"
"$OURS/printf" '%f|%6F|%-6f|%+f|\n' -nan -NAN nan nan > out 2> err
eq "printf: a NaN keeps its sign, in capitals for F, to the field width" "-nan|  -NAN|nan   |+nan| 0" "$(cat out) $(wc -c < err | tr -d ' ')"
LC_ALL=C LC_NUMERIC=C "$OURS/printf" '%.1f\n' 1.5 > out
eq "printf %f: the radix character in the C locale" "1.5" "$(cat out)"
if LC_ALL=de_DE.UTF-8 "$OURS/printf" '%.1f' 1,5 2> /dev/null | grep -q ','; then
	eq "printf %f: LC_NUMERIC gives the radix character, in the output and in the argument" "1,5 0" "$(LC_ALL=de_DE.UTF-8 "$OURS/printf" '%.1f' 1,5 2> err) $(wc -c < err | tr -d ' ')"
	eq "printf %f: LC_NUMERIC alone is enough" "2,5" "$(LC_ALL= LC_NUMERIC=de_DE.UTF-8 "$OURS/printf" '%.1f' 2,5 2> /dev/null)"
else
	echo "skip: no locale de_DE.UTF-8"
fi
"$OURS/printf" 2> err
eq "printf with no operand is a usage error" "2 1" "$? $(wc -l < err | tr -d ' ')"
"$OURS/printf" -x 2> /dev/null
eq "printf -x is a usage error (it has no options)" 2 $?
"$OURS/printf" -- '%s\n' -x > out
eq "printf -- format -x: the -- ends the options, -x is an argument" "0 -x" "$? $(cat out)"
"$OURS/printf" -- '-%d\n' 5 > out
eq "printf -- '-%d': a format that starts with -" "-5" "$(cat out)"
"$OURS/printf" '%s\n' -- a > out
eq "printf format -- a: after the format, -- is an argument" "--/a/" "$(tr '\n' '/' < out)"
"$OURS/printf" '%s' '' > out
eq "printf %s of an empty string: nothing" "0 0" "$? $(wc -c < out | tr -d ' ')"
"$OURS/printf" '' x y > out
eq "printf of an empty format: nothing" "0 0" "$? $(wc -c < out | tr -d ' ')"
"$OURS/printf" '%.3s|%5s|%-5s|%c\n' abcdef ab ab xyz > out
eq "printf: width, precision and %c" "abc|   ab|ab   |x" "$(cat out)"
"$OURS/printf" '%+d|% d|%05d|%.3d|%#o|%#x|%u\n' 5 5 5 5 8 255 -1 > out
eq "printf: flags of the integer conversions" "+5| 5|00005|005|010|0xff|18446744073709551615" "$(cat out)"
awk 'BEGIN { for (i = 0; i < 40000; i++) printf "%d\n", i }' > nums
"$OURS/printf" '%d,' $(cat nums) > out
eq "printf: the format used 40000 times" "40000 39999," "$(tr -cd ',' < out | wc -c | tr -d ' ') $(tr ',' '\n' < out | sed -n 40000p),"
head -c 100000 /dev/zero | tr '\0' x > big
"$OURS/printf" '%s|' "$(cat big)" > out
eq "printf: an argument of 100000 bytes" 100001 "$(wc -c < out | tr -d ' ')"
"$OURS/printf" '%100000d' 7 > out
eq "printf: a field width of 100000" 100000 "$(wc -c < out | tr -d ' ')"
"$OURS/printf" 'x' >&- 2> /dev/null
eq "printf with stdout closed: status" 1 $?
"$OURS/printf" 'x' <&- > /dev/null
eq "printf with stdin closed: fine, it reads nothing" 0 $?
"$OURS/printf" '%s\n' $(cat nums) | head -n 2 > out
eq "printf into a closed pipe ends quietly" "0/1/" "$(tr '\n' '/' < out)"
if LC_ALL=$U8 "$OURS/printf" '%d' "'$(printf '\303\251')" 2> /dev/null | grep -q '^233$'; then
	eq "printf %d '<e acute> in a UTF-8 locale: the character's value" 233 "$(LC_ALL=$U8 "$OURS/printf" '%d' "'$(printf '\303\251')")"
else
	echo "skip: no locale $U8"
fi
eq "printf in the C locale: the first byte of a multibyte character" 195 "$(LC_ALL=C "$OURS/printf" '%d' "'$(printf '\303\251')")"
"$OURS/printf" 'a%+.2147483647db' 1 > out 2> err
eq "printf: a result longer than INT_MAX bytes is an error, the rest is written" "1 ab 1" "$? $(cat out) $(wc -l < err | tr -d ' ')"

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
# DESCRIPTION: no newline immediately before a <carriage-return> or <backspace>, -s included
eq "fold -s -w 4: no newline before a return; the blank in front of it is not used" "616263200d646566670a68690a" "$(printf 'abc \rdefghi\n' | "$OURS/fold" -s -w 4 | hex)"
eq "fold -s -w 4: the same before a backspace" "6162632008640a656667680a690a" "$(printf 'abc \bdefghi\n' | "$OURS/fold" -s -w 4 | hex)"
eq "fold -s -w 4: an earlier blank is used when the last one stands before a return" "61200a62200d636465660a670a" "$(printf 'a b \rcdefg\n' | "$OURS/fold" -s -w 4 | hex)"
eq "fold -bs -w 4: with -b a return is a byte like the rest" "616263200a0d6465660a670a" "$(printf 'abc \rdefg\n' | "$OURS/fold" -bs -w 4 | hex)"
# -b counts bytes but never cuts a character in two (a UTF-8 locale is needed to know what one is)
if [ "$(LC_ALL=$U8 locale charmap 2> /dev/null)" = UTF-8 ]; then
	eq "fold -b -w 2: two two-byte characters, one per line" "c3a90ac3a90a" "$(printf '\303\251\303\251\n' | LC_ALL=$U8 "$OURS/fold" -b -w 2 | hex)"
	eq "fold -b -w 3: a character that does not fit goes to the next line whole" "61c3a90ac3a90a" "$(printf 'a\303\251\303\251\n' | LC_ALL=$U8 "$OURS/fold" -b -w 3 | hex)"
	eq "fold -w 2: a wide character counts two columns" "e697a50ae69cac0a" "$(printf '\346\227\245\346\234\254\n' | LC_ALL=$U8 "$OURS/fold" -w 2 | hex)"
fi
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
if [ "$(LC_ALL=$U8 locale charmap 2> /dev/null)" = UTF-8 ]; then
	eq "fold: a UTF-8 character cut by the end of a read stays whole" "e697a50a65" "$({ printf '\346\227'; sleep 1; printf '\245e\n'; } | LC_ALL=$U8 "$OURS/fold" -w 2 | hex | cut -c1-10)"
	eq "fold: two CJK characters need four columns" "e697a50ae697a5" "$(printf '\346\227\245\346\227\245' | LC_ALL=$U8 "$OURS/fold" -w 3 | hex)"
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
eq "cut -n: a start inside a character moves back" "c3a90a" "$(printf '\303\251\n' | LC_ALL=$U8 "$OURS/cut" -n -b 2 | hex)"
eq "cut -n: an end inside a character drops the character" "0a" "$(printf '\303\251\n' | LC_ALL=$U8 "$OURS/cut" -n -b 1 | hex)"
eq "cut -b may cut a character in two" "c30a" "$(printf '\303\251\n' | LC_ALL=$U8 "$OURS/cut" -b 1 | hex)"
eq "cut -c takes a whole character" "c3a90a" "$(printf '\303\251\n' | LC_ALL=$U8 "$OURS/cut" -c 1 | hex)"
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
deeptree deep 700 1
"$OURS/rm" -r deep 2> e
eq "rm -r of a tree 700 levels deep (over 11000 bytes of path)" "0 gone" "$? $([ -e deep ] || echo gone)"
mkdir deep
deeptree deep 300 1
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
mt() { st_mtime_s "$1"; }
at() { st_atime_s "$1"; }

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
if TZ=UTC0 /usr/bin/touch -t 190001010000 probe 2> /dev/null && [ "$(mt probe)" = -2208988800 ]; then
	TZ=UTC0 "$OURS/touch" -t 190001010000 f
	eq "touch -t 190001010000: before the Epoch is fine where the file system holds it" -2208988800 "$(mt f)"
else
	echo "skip: this file system does not hold times before 1901"
fi
rm -f probe
"$OURS/touch" -d 1969-12-31T23:59:59Z f
eq "touch -d 1969-12-31T23:59:59Z" -1 "$(mt f)"
TZ=UTC0 "$OURS/touch" -t 203801010000 f
eq "touch -t 203801010000 (the spec's minimum)" 2145916800 "$(mt f)"
TZ=UTC0 "$OURS/touch" -t 210001010000 f
eq "touch -t 210001010000" 4102444800 "$(mt f)"

# fractions of a second
"$OURS/touch" -d 2007-11-12T10:15:30.123456789Z f
if st_mtime f > /dev/null 2>&1; then
	eq "touch -d keeps nanoseconds" 1194862530.123456789 "$(st_mtime f)"
	"$OURS/touch" -d 2007-11-12T10:15:30,5Z f
	eq "touch -d accepts a comma fraction" 1194862530.500000000 "$(st_mtime f)"
	"$OURS/touch" -d 2007-11-12T10:15:30.9999999999Z f
	eq "touch -d cuts a fraction beyond nanoseconds" 1194862530.999999999 "$(st_mtime f)"
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
eq "TZ=JST-9 is nine hours ahead of -u" 9 "$(a=$(TZ=JST-9 "$OURS/date" +%H); b=$("$OURS/date" -u +%H); echo $(( (${a#0} - ${b#0} + 24) % 24 )))"
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

#----------------------------------------------------------------------- cp
fresh
echo hello > a
"$OURS/cp" a b
eq "cp a b: status 0 and the same contents" "0 hello" "$? $(cat b)"
eq "cp: a new file has the mode of the source less the umask 022" "$(mode a)" "$(mode b)"
chmod 751 a
(umask 077; "$OURS/cp" a um077)
eq "cp under umask 077: mode 700 from 751" -rwx------ "$(mode um077)"
(umask 0; "$OURS/cp" a um0)
eq "cp under umask 0: mode 751" -rwxr-x--x "$(mode um0)"
chmod 4755 a
"$OURS/cp" a sid
eq "cp: the set-user-ID bit is not copied" -rwxr-xr-x "$(mode sid)"
chmod 644 a
echo old > keep; chmod 600 keep
"$OURS/cp" a keep
eq "cp over an existing file: contents replaced, its mode kept" "hello -rw-------" "$(cat keep) $(mode keep)"
rm -f b um077 um0 sid keep

# the destination name rules
mkdir dir
echo second > a2
"$OURS/cp" a a2 dir
eq "cp a a2 dir: both land in dir" "a a2" "$(ls dir | tr '\n' ' ' | sed 's/ $//')"
"$OURS/cp" a dir/
eq "cp a dir/: into the directory" hello "$(cat dir/a)"
"$OURS/cp" a dir/x/ 2> /dev/null
eq "cp a dir/x/ (x does not exist): an error" 1 $?
"$OURS/cp" a a2 nosuchdir 2> e
eq "cp a a2 nosuchdir: status 1, nothing made" "1 absent" "$? $([ -e nosuchdir ] || echo absent)"
echo f > plainf
"$OURS/cp" a a2 plainf 2> e
eq "cp a a2 plainf: not a directory, status 1, plainf untouched" "1 f" "$? $(cat plainf)"
"$OURS/cp" a 2> e
eq "cp with one operand: usage" "2 usage" "$? $(sed 's/:.*//;q' e)"
"$OURS/cp" 2> /dev/null
eq "cp with no operand: usage, status 2" 2 $?
"$OURS/cp" -Q a x 2> /dev/null
eq "cp -Q: usage, status 2" 2 $?
rm -rf dir plainf a2

# -i reads its line byte by byte
echo new > n1; echo old1 > t1; echo old2 > t2
eq "cp -i: the answer line only is read from a shared standard input" "REST" "$(printf 'y\nREST\n' | { "$OURS/cp" -i n1 t2 2> /dev/null; cat; })"
echo old1 > t1
printf 'n\ny\n' | "$OURS/cp" -i n1 t1 2> e
eq "cp -i with n: the file stays, status 0" "0 old1" "$? $(cat t1)"
eq "cp -i: the prompt names the destination" 1 "$(grep -c 't1' e)"
"$OURS/cp" -i n1 t1 < /dev/null 2> /dev/null
eq "cp -i: end of file is a no" "0 old1" "$? $(cat t1)"
"$OURS/cp" -i n1 t1 <&- 2> /dev/null
eq "cp -i: a closed standard input is a no" "0 old1" "$? $(cat t1)"
printf 'y\n' | "$OURS/cp" -i n1 t1 2> /dev/null
eq "cp -i with y: copied" new "$(cat t1)"
printf 'n\nn\nn\n' | "$OURS/cp" -i n1 fresh_name 2> e
eq "cp -i to a new name: no question" "0 0 new" "$? $(wc -c < e | tr -d ' ') $(cat fresh_name)"
rm -f n1 t1 t2 fresh_name

# not truncating what cannot be replaced by something better
echo precious > dst
echo secret > unread; chmod 000 unread
"$OURS/cp" unread dst 2> e
eq "cp of an unreadable source: status 1, the destination keeps its contents" "1 precious" "$? $(cat dst)"
chmod 644 unread; rm -f unread dst

# a failed source does not stop the others
echo b > srcb; mkdir into
"$OURS/cp" nosuch srcb into 2> e
eq "cp nosuch srcb into: status 1, srcb is copied" "1 b" "$? $(cat into/srcb)"
rm -rf into srcb

# a write error: the file size limit, with SIGXFSZ ignored so that write() reports EFBIG
head -c 200000 /dev/zero > big
(trap '' XFSZ; ulimit -f 20 2> /dev/null; "$OURS/cp" big small_out 2> e; echo $? > st)
eq "cp when the file size limit stops the write: status 1 and a diagnostic" "1 1" "$(cat st) $([ -s e ] && echo 1)"
(trap '' XFSZ; ulimit -f 20 2> /dev/null; "$OURS/cp" big out1 big_too 2> e; echo $? > st) 2> /dev/null
rm -f small_out big_too out1 st

# big and odd contents
head -c 3000000 /dev/urandom > rnd
"$OURS/cp" rnd rnd2
eq "cp of 3 MB of random bytes is identical" 0 "$(cmp rnd rnd2 > /dev/null; echo $?)"
printf 'a\0b\0\0\n' > nul
"$OURS/cp" nul nul2
eq "cp keeps NUL bytes" 0 "$(cmp nul nul2 > /dev/null; echo $?)"
: > empty
"$OURS/cp" empty empty2
eq "cp of an empty file" "0 0" "$? $(wc -c < empty2 | tr -d ' ')"
rm -f rnd rnd2 nul nul2 empty empty2

# special names and special destinations
echo dash > ./-
"$OURS/cp" -- - dashcopy
eq "cp -- - x: \"-\" is a file name" dash "$(cat dashcopy)"
echo data > dd
"$OURS/cp" dd /dev/null
eq "cp onto /dev/null" 0 $?
"$OURS/cp" /dev/null emptied
eq "cp /dev/null x makes an empty regular file" "0 0 file" "$? $(wc -c < emptied | tr -d ' ') $([ -f emptied ] && echo file)"
mkfifo ff
(echo through-the-fifo > ff &)
"$OURS/cp" ff fromfifo
eq "cp of a FIFO without -R reads it as data" "through-the-fifo" "$(cat fromfifo)"
"$OURS/cp" -R ff fifocopy
eq "cp -R of a FIFO makes a FIFO" 1 "$([ -p fifocopy ] && echo 1)"
rm -f ./- dashcopy dd emptied ff fromfifo fifocopy

# -p
echo p > pf; chmod 640 pf
"$OURS/touch" -d 2001-01-01T00:00:00Z pf
"$OURS/cp" -p pf pf2
eq "cp -p: times and mode" "$(st_mtime_s pf) $(st_mode pf)" "$(st_mtime_s pf2) $(st_mode pf2)"
"$OURS/cp" pf pf3
eq "cp without -p: the copy is new" 1 "$([ "$(st_mtime_s pf3)" -gt 978307200 ] && echo 1)"
rm -f pf pf2 pf3

# hierarchies
mkdir -p t/a/b t/c; echo 1 > t/a/b/f; echo 2 > t/c/g; echo 3 > t/top; : > t/c/empty
ln -s top t/lnk; ln -s ../c t/a/up; ln -s nowhere t/dang
ln t/top t/hard1; ln t/top t/hard2
"$OURS/cp" -R t tc 2> e
eq "cp -R: status 0, silent" "0 0" "$? $(wc -c < e | tr -d ' ')"
eq "cp -R: the same names" "$(cd t && find . | sort)" "$(cd tc && find . | sort)"
eq "cp -R: links are copied as links (the default)" "top ../c nowhere" "$(readlink tc/lnk) $(readlink tc/a/up) $(readlink tc/dang)"
eq "cp -R: hard links are not kept (three files)" "1 1 1" "$(ls -l tc/top tc/hard1 tc/hard2 | awk '{print $2}' | tr '\n' ' ' | sed 's/ $//' | sed 's/ /,/g;s/,/ /g')"
eq "cp -R: contents" "1 2 3" "$(cat tc/a/b/f tc/c/g tc/top | tr '\n' ' ' | sed 's/ $//')"
"$OURS/cp" -RL t tl 2> e
eq "cp -RL: the dangling link in the tree is the one error" "1 1" "$? $(wc -l < e | tr -d ' ')"
eq "cp -RL: the other links were followed" "regular regular" "$([ -f tl/lnk ] && [ ! -L tl/lnk ] && echo regular) $([ -d tl/a/up ] && [ ! -L tl/a/up ] && echo regular)"
rm -rf tc tl
"$OURS/cp" -R t/ tslash
eq "cp -R t/ x" "$(cd t && find . | sort)" "$(cd tslash && find . | sort)"
mkdir existing
"$OURS/cp" -R t existing
eq "cp -R t existing: lands below it" "$(cd t && find . | sort)" "$(cd existing/t && find . | sort)"
"$OURS/cp" -R t/. existing2 2> /dev/null; mkdir existing3
"$OURS/cp" -R t/. existing3
eq "cp -R t/. dir: the contents, not t itself" "$(cd t && find . | sort)" "$(cd existing3 && find . | sort)"
rm -rf tslash existing existing2 existing3

# a directory is never copied into itself; nothing is created
"$OURS/cp" -R t t/inside 2> e
eq "cp -R t t/inside: refused, nothing made" "1 absent" "$? $([ -e t/inside ] || echo absent)"
"$OURS/cp" -R t t 2> e
eq "cp -R t t: refused (it would go to t/t)" "1 absent" "$? $([ -e t/t ] || echo absent)"
"$OURS/cp" -R . t/dot 2> /dev/null
eq "cp -R . t/dot: refused" "1 absent" "$? $([ -e t/dot ] || echo absent)"
ln -s . t/selfloop
"$OURS/cp" -RL t tloop 2> /dev/null
eq "cp -RL over a link to the directory itself ends, with an error" 1 $?
rm -rf tloop
"$OURS/cp" -RL t tloop 2> e
eq "cp -RL over that link: one 'causes a cycle' diagnostic, no copy of the link" "1 1 absent" "$? $(grep -c 'causes a cycle' e) $([ -e tloop/selfloop ] || echo absent)"
rm -rf tloop t/selfloop

# read-only directories are copied, and end up read-only
mkdir -p ro/inner; echo x > ro/inner/f; chmod 555 ro/inner ro
"$OURS/cp" -R ro roc 2> e
eq "cp -R of read-only directories: status 0" "0 x" "$? $(cat roc/inner/f)"
eq "cp -R: the read-only modes are kept" "dr-xr-xr-x dr-xr-xr-x" "$(ls -ld roc | cut -c1-10) $(ls -ld roc/inner | cut -c1-10)"
chmod -R u+rwx ro roc; rm -rf ro roc
(umask 077; "$OURS/cp" -R t tum)
eq "cp -R under umask 077: directories 700" "drwx------ drwx------" "$(ls -ld tum | cut -c1-10) $(ls -ld tum/a/b | cut -c1-10)"
rm -rf tum

# a destination in the way
mkdir blocker; echo file > blocker/a
"$OURS/cp" -R t blocker 2> e
eq "cp -R t blocker: blocker/t is made; blocker/a (a file) does not matter" "0" "$?"
rm -rf blocker
mkdir dst2; echo file > dst2/a
"$OURS/cp" -R t/. dst2 2> e
eq "cp -R t/. dst2 where dst2/a is a file: the one error, the rest copied" "1 1 3" "$? $(wc -l < e | tr -d ' ') $(cat dst2/top)"
rm -rf dst2
echo notadir > notadir
"$OURS/cp" -R t notadir 2> e
eq "cp -R t notadir: error, the file is left" "1 notadir" "$? $(cat notadir)"
rm -f notadir

# deep and wide
mkdir deep
(
	cd deep || exit 1
	i=0
	while [ $i -lt 200 ]; do
		mkdir dd || exit 1
		cd dd || exit 1
		: > f
		i=$((i + 1))
	done
)
"$OURS/cp" -R deep deepc 2> e
eq "cp -R of a tree 200 levels deep" "0" "$?"
eq "cp -R of a deep tree: same shape" "$(cd deep && find . | wc -l | tr -d ' ')" "$(cd deepc && find . | wc -l | tr -d ' ')"
rm -rf deepc
(ulimit -n 12; exec "$OURS/cp" -R deep deepc2 2> e); echo $? > st
eq "cp -R with 12 descriptors allowed: still fine (no descriptor per level)" "0" "$(cat st)"
rm -rf deep deepc2 st
mkdir longp
# longer than PATH_MAX here, whatever it is (1024 on macOS, 4096 on Linux)
deeptree longp $(($(getconf PATH_MAX /) / 17 + 20)) 0
"$OURS/cp" -R longp longpc 2> e
eq "cp -R of a tree deeper than PATH_MAX: an error, not a crash" "1" "$?"
eq "cp -R of a tree deeper than PATH_MAX: a diagnostic" 1 "$([ -s e ] && echo 1)"
rm -rf longp longpc
mkdir wide
( cd wide && i=0 && while [ $i -lt 3000 ]; do : > "file$i"; i=$((i + 1)); done )
"$OURS/cp" -R wide widec
eq "cp -R of a directory with 3000 entries" "3000" "$(ls widec | wc -l | tr -d ' ')"
rm -rf wide widec

# descriptors
echo x > cs
"$OURS/cp" cs cs2 >&-
eq "cp with stdout closed" "0 x" "$? $(cat cs2)"
"$OURS/cp" nosuch cs3 2>&-
eq "cp with stderr closed and an error: status 1" 1 $?
"$OURS/cp" cs cs4 <&-
eq "cp with stdin closed" "0 x" "$? $(cat cs4)"
rm -rf cs cs2 cs4 t

#----------------------------------------------------------------------- ls
fresh
mkdir d1 d2; : > f1; : > f2; : > d1/a; : > d1/b; : > d2/c; : > .hid
eq "ls: files first, then directories, each sorted" "f1 f2 d1: a b d2: c" "$("$OURS/ls" d2 f2 d1 f1 | tr '\n' ' ' | sed 's/ $//;s/  */ /g')"
eq "ls with several operands: a blank line before each header but the first" "f1
f2

d1:
a
b

d2:
c" "$("$OURS/ls" f1 f2 d1 d2)"
eq "ls dir1 dir2: the header is the first line, no blank line before it" "d1:" "$("$OURS/ls" d1 d2 | head -1)"
eq "ls of one directory has no header" "a
b" "$("$OURS/ls" d1)"
eq "ls with no operand lists dot without the hidden file" "d1 d2 f1 f2" "$("$OURS/ls" | tr '\n' ' ' | sed 's/ $//')"
eq "ls -a includes . .. and the hidden file" ". .. .hid d1 d2 f1 f2" "$("$OURS/ls" -a | tr '\n' ' ' | sed 's/ $//')"
eq "ls -A leaves out . and .." ".hid d1 d2 f1 f2" "$("$OURS/ls" -A | tr '\n' ' ' | sed 's/ $//')"
eq "ls .hid: a hidden file named as an operand is listed" ".hid" "$("$OURS/ls" .hid)"
eq "ls -d . and ls -d d1" ". d1" "$("$OURS/ls" -d . d1 | tr '\n' ' ' | sed 's/ $//')"
eq "ls -R: exact output" "d1:
a
b

d2:
c" "$("$OURS/ls" -R d1 d2 | sed -n '1,100p')"
mkdir -p t/a/b t/c; : > t/a/b/f; : > t/top
eq "ls -R t: every directory has a header, the first without a blank line before it" "t:
a
c
top

t/a:
b

t/a/b:
f

t/c:" "$("$OURS/ls" -R t)"
eq "ls -R with one empty directory still has its header" "t/c:" "$("$OURS/ls" -R t/c)"
eq "ls dir with only hidden files: no output at all" "" "$(mkdir oh; : > oh/.x; "$OURS/ls" oh)"
rmdir oh 2> /dev/null; rm -rf oh
"$OURS/ls" nosuch f1 2> e > /dev/null
eq "ls nosuch f1: status 1, f1 is listed, one diagnostic naming it" "1 f1 1" "$? $("$OURS/ls" nosuch f1 2> /dev/null) $(grep -c nosuch e)"
"$OURS/ls" -d 2> /dev/null > /dev/null
eq "ls -d with no operand lists dot" "." "$("$OURS/ls" -d)"

# -i
ino=$(st_ino f1)
eq "ls -i: the inode number first" "$ino f1" "$("$OURS/ls" -i f1)"
eq "ls -li: the inode number before the mode" 1 "$("$OURS/ls" -li f1 | grep -c "^$ino -rw")"
eq "ls -1i d1 pads the inode numbers to one width" 1 "$("$OURS/ls" -1i d1 | awk '{print length($1)}' | sort -u | wc -l | tr -d ' ')"

# -q and the names
: > "$(printf 'tab\there')"
: > "$(printf 'nl\nhere')"
: > "$(printf 'ctl\001x')"
eq "ls -q: tab, newline and a control character are question marks" "ctl?x
nl?here
tab?here" "$("$OURS/ls" -q | grep -E 'ctl|nl|tab')"
eq "ls without -q (not a terminal): the bytes as they are" 1 "$("$OURS/ls" | grep -c "$(printf 'tab\there')")"
eq "ls -q: a name with a blank is not changed" "a b" "$(: > 'a b'; "$OURS/ls" -q 'a b')"
rm -f 'a b'
: > "$(printf '\303\251')"
eq "ls -q in a UTF-8 locale keeps a printable multibyte character" "$(printf '\303\251')" "$(LC_ALL=$U8 "$OURS/ls" -q "$(printf '\303\251')")"
eq "ls -q in the C locale: its two bytes are not printable" "??" "$(LC_ALL=C "$OURS/ls" -q "$(printf '\303\251')")"
rm -f "$(printf '\303\251')" "$(printf 'tab\there')" "$(printf 'nl\nhere')" "$(printf 'ctl\001x')"

# columns
mkdir cols; ( cd cols; for n in aaa bbb ccc ddd eee fff ggg hhh iii jjj kkk lll; do : > $n; done )
eq "ls -C with COLUMNS=40: eight 5-wide columns fit, 12 names make 2 rows, down the columns" "aaa  ccc  eee  ggg  iii  kkk
bbb  ddd  fff  hhh  jjj  lll" "$(COLUMNS=40 "$OURS/ls" -C cols)"
eq "ls -C with COLUMNS=20 on 12 names: 4 columns, 3 rows" "aaa  ddd  ggg  jjj
bbb  eee  hhh  kkk
ccc  fff  iii  lll" "$(COLUMNS=20 "$OURS/ls" -C cols)"
eq "ls -x with COLUMNS=20: across the rows" "aaa  bbb  ccc  ddd
eee  fff  ggg  hhh
iii  jjj  kkk  lll" "$(COLUMNS=20 "$OURS/ls" -x cols)"
eq "ls -C with COLUMNS=5: too narrow, one column" 12 "$(COLUMNS=5 "$OURS/ls" -C cols | wc -l | tr -d ' ')"
eq "ls -C with COLUMNS=abc: falls back (80), all on one line" 1 "$(COLUMNS=abc "$OURS/ls" -C cols | wc -l | tr -d ' ')"
eq "ls -C with COLUMNS=0: falls back" 1 "$(COLUMNS=0 "$OURS/ls" -C cols | wc -l | tr -d ' ')"
eq "ls -C with COLUMNS=40 and 80 give different output" 1 "$([ "$(COLUMNS=40 "$OURS/ls" -C cols)" != "$(COLUMNS=80 "$OURS/ls" -C cols)" ] && echo 1)"
( cd cols; : > averyveryverylongfilename_that_is_long )
eq "ls -C: a long name is not cut" 1 "$(COLUMNS=30 "$OURS/ls" -C cols | grep -c averyveryverylongfilename_that_is_long)"
eq "ls -C: one constant column width, the second column starts at the same place in every row" 1 "$(COLUMNS=100 "$OURS/ls" -C cols | awk 'NF>1{print index($0,$2)}' | sort -u | wc -l | tr -d ' ')"
rm -f cols/averyveryverylongfilename_that_is_long
eq "ls -m: names separated by comma and space" "aaa, bbb, ccc, ddd, eee, fff, ggg, hhh, iii, jjj, kkk, lll" "$("$OURS/ls" -m cols)"
eq "ls -m with COLUMNS=30: wraps, no line longer than 30" "0" "$(COLUMNS=30 "$OURS/ls" -m cols | awk 'length($0)>30' | wc -l | tr -d ' ')"
eq "ls -m with COLUMNS=30: wraps after a comma" "aaa, bbb, ccc, ddd, eee, fff,
ggg, hhh, iii, jjj, kkk, lll" "$(COLUMNS=30 "$OURS/ls" -m cols)"
eq "ls -m of an empty directory" "" "$(mkdir em; "$OURS/ls" -m em)"
rmdir em
eq "ls -1 forces one per line" 12 "$("$OURS/ls" -1 cols | wc -l | tr -d ' ')"
eq "ls not on a terminal: one per line" 12 "$("$OURS/ls" cols | wc -l | tr -d ' ')"
eq "ls -l -1 is the same as -l" "$("$OURS/ls" -l cols)" "$("$OURS/ls" -l -1 cols)"
eq "ls -1 -l is the same as -l" "$("$OURS/ls" -l cols)" "$("$OURS/ls" -1 -l cols)"
eq "ls -C then -l is long, -l then -C is not" "1 0" "$("$OURS/ls" -Cl cols | grep -c '^total') $("$OURS/ls" -lC cols | grep -c '^total')"
rm -rf cols

# the terminal
if [ -x /usr/bin/script ] && /usr/bin/script -q /dev/null true < /dev/null > /dev/null 2>&1; then
	mkdir tty; ( cd tty; for n in aaa bbb ccc; do : > $n; done; : > "$(printf 'x\ty')" )
	export OURS
	script -q /dev/null sh -c 'cd tty && "$OURS/ls"' < /dev/null > ty.out 2>&1
	eq "ls on a terminal: several names on a line" 1 "$([ "$(sed -n 1p ty.out | tr -d '\r' | wc -w | tr -d ' ')" -ge 4 ] && echo 1)"
	eq "ls on a terminal: a tab in a name is a question mark" 1 "$(grep -c 'x?y' ty.out)"
	script -q /dev/null sh -c 'cd tty && "$OURS/ls" -1' < /dev/null > ty.out 2>&1
	eq "ls -1 on a terminal: one per line" 4 "$(tr -d '\r' < ty.out | grep -c .)"
	rm -rf tty ty.out
else
	echo "skip: ls on a terminal"
fi

# times
rm -rf tm; mkdir tm; cd tm
: > old; : > fut; : > new
"$OURS/touch" -d 2001-01-01T00:00:00Z old
"$OURS/touch" -t 209901020304 fut
eq "ls -l: a file older than six months shows the year, two spaces after the day" 1 "$(TZ=UTC0 "$OURS/ls" -l old | grep -c ' Jan  1  2001 old$')"
eq "ls -l: a file in the future shows the year" 1 "$(TZ=UTC0 "$OURS/ls" -l fut | grep -c ' Jan  2  2099 fut$')"
want=$(TZ=UTC0 date '+%b %e %H:%M')
eq "ls -l: a new file shows the month, day and time, as date +%b %e %H:%M" 1 "$(TZ=UTC0 "$OURS/ls" -l new | grep -c " $want new\$")"
eq "ls -l: TZ moves the date (2001-01-01T00:00Z is Dec 31 2000 in New York)" "Dec 31 2000 Jan 1 2001" "$(TZ=EST5EDT "$OURS/ls" -l old | awk '{print $6,$7,$8}') $(TZ=UTC0 "$OURS/ls" -l old | awk '{print $6,$7,$8}')"
"$OURS/touch" -d 2001-06-15T12:00:00Z -a old 2> /dev/null
eq "ls -lu shows the access time, ls -l the modification time" "Jun 15 2001 Jan 1 2001" "$(TZ=UTC0 "$OURS/ls" -lu old | awk '{print $6,$7,$8}') $(TZ=UTC0 "$OURS/ls" -l old | awk '{print $6,$7,$8}')"
eq "ls -lc shows the change time (now, so no year)" 1 "$(TZ=UTC0 "$OURS/ls" -lc old | awk '$8 ~ /:/ {print 1}')"
"$OURS/touch" -d 2005-05-05T00:00:00Z -m old
: > o2; "$OURS/touch" -d 2003-03-03T00:00:00Z o2
eq "ls -t: newest first (the file dated 2099 is newer than now)" "fut new old o2" "$("$OURS/ls" -t | tr '\n' ' ' | sed 's/ $//')" 2> /dev/null
eq "ls -tr: oldest first" "o2 old new fut" "$("$OURS/ls" -tr | head -4 | tr '\n' ' ' | sed 's/ $//')" 2> /dev/null
"$OURS/touch" -a -d 2010-01-01T00:00:00Z o2
"$OURS/touch" -a -d 2009-01-01T00:00:00Z old
eq "ls -tu: by access time, newest first" "o2 old" "$("$OURS/ls" -tu o2 old | tr '\n' ' ' | sed 's/ $//')"
eq "ls -t: by modification time, newest first" "old o2" "$("$OURS/ls" -t o2 old | tr '\n' ' ' | sed 's/ $//')"
cd ..; rm -rf tm

# sizes, blocks and the total line
mkdir sz; ( cd sz; head -c 51200 /dev/zero > big; : > small )
sleep 1
eq "ls -l shows the size in bytes" "51200 0" "$("$OURS/ls" -l sz | awk '$NF=="big"{print $5} $NF=="small"{print $5}' | tr '\n' ' ' | sed 's/ $//')"
t512=$("$OURS/ls" -l sz | sed -n 's/^total //p')
t1k=$("$OURS/ls" -lk sz | sed -n 's/^total //p')
eq "ls -l: total is at least 100 blocks of 512 bytes for a 51200 byte file" 1 "$([ "$t512" -ge 100 ] && echo 1)"
eq "ls -lk: the total is in 1024 byte blocks, half of the 512 byte one rounded up" "$(( (t512 + 1) / 2 ))" "$t1k"
eq "ls -s: blocks before the name, -k halves them" "$("$OURS/ls" -s sz | awk '$2=="big"{print $1}')" "$(( $("$OURS/ls" -sk sz | awk '$2=="big"{print $1}') * 2 ))"
eq "ls -s has a total line" 1 "$("$OURS/ls" -s sz | grep -c '^total')"
eq "ls -l big: no total line for a file operand" 0 "$("$OURS/ls" -l sz/big | grep -c '^total')"
eq "ls -S: biggest first" "big small" "$("$OURS/ls" -S sz | tr '\n' ' ' | sed 's/ $//')"
eq "ls -Sr: smallest first" "small big" "$("$OURS/ls" -Sr sz | tr '\n' ' ' | sed 's/ $//')"
rm -rf sz

# the long format
mkdir lf; cd lf
echo content > file; ln -s file lnk; ln -s nowhere dang; mkfifo ff; mkdir sub
chmod 640 file
u=$(id -un); gname=$(id -gn)
eq "ls -l file: the seven fields" "-rw-r----- 1 $u $gname 8" "$("$OURS/ls" -l file | awk '{print $1,$2,$3,$4,$5}')"
eq "ls -l shows a symbolic link with its contents" 1 "$("$OURS/ls" -l lnk | grep -c ' lnk -> file$')"
eq "ls -l shows a dangling link too" 1 "$("$OURS/ls" -l dang | grep -c ' dang -> nowhere$')"
eq "ls -lL: the information of the target, the name of the link, no arrow" "-rw-r-----" "$("$OURS/ls" -lL lnk | awk '{print $1}')"
eq "ls -lL does not print an arrow" 0 "$("$OURS/ls" -lL lnk | grep -c -- '->')"
eq "ls -lL on a dangling link is an error" 1 "$("$OURS/ls" -lL dang > /dev/null 2>&1; echo $?)"
eq "ls -ld: the entry type characters for a pipe, a file, a link and a directory" "p-ld" "$("$OURS/ls" -ld ff file lnk sub | awk '{printf "%s", substr($1,1,1)}')"
eq "ls -n shows numbers" "$(id -u) $(id -g)" "$("$OURS/ls" -n file | awk '{print $3,$4}')"
eq "ls -g leaves out the owner, ls -o the group" "$gname $u" "$("$OURS/ls" -g file | awk '{print $3}') $("$OURS/ls" -o file | awk '{print $3}')"
eq "ls -F: the signs" "ff| file lnk@ sub/" "$("$OURS/ls" -F | tr '\n' ' ' | sed 's/ $//' | sed 's/dang@ //')"
chmod 755 file
eq "ls -F: * for an executable" "file*" "$("$OURS/ls" -F file)"
eq "ls -p: a slash for directories only" "ff file lnk sub/" "$("$OURS/ls" -p | grep -v dang | tr '\n' ' ' | sed 's/ $//')"
eq "ls -lF: the sign goes after the name, before the arrow" 1 "$("$OURS/ls" -lF lnk | grep -c ' lnk@ -> file$')"
cd ..; rm -rf lf

# symbolic link operands
mkdir ld; : > ld/in; ln -s ld lnk; ln -s ld/in flnk
eq "ls lnk (to a directory): its contents" "in" "$("$OURS/ls" lnk)"
eq "ls -d lnk: the link itself" "lnk" "$("$OURS/ls" -d lnk)"
eq "ls -F lnk: the link itself, with @" "lnk@" "$("$OURS/ls" -F lnk)"
eq "ls -l lnk: the link itself" 1 "$("$OURS/ls" -l lnk | grep -c ' lnk -> ld$')"
eq "ls -p lnk: followed (-p does not stop it)" "in" "$("$OURS/ls" -p lnk)"
eq "ls -H -l lnk: followed" "in" "$("$OURS/ls" -H -l lnk | awk 'NR>1{print $NF}')"
eq "ls -L -F lnk: followed" "in" "$("$OURS/ls" -L -F lnk)"
eq "ls -Hd lnk: the information of the target, the name of the link" "lnk" "$("$OURS/ls" -Hd lnk)"
eq "ls -ld -H lnk shows a directory" "d" "$("$OURS/ls" -ld -H lnk | cut -c1)"
eq "ls -ld lnk shows a link" "l" "$("$OURS/ls" -ld lnk | cut -c1)"
eq "ls -LH and -HL do not fail" "0 0" "$("$OURS/ls" -LH lnk > /dev/null; echo $?) $("$OURS/ls" -HL lnk > /dev/null; echo $?)"
eq "ls -R does not follow a link to a directory inside" 0 "$(mkdir -p rr; ln -s ../ld rr/l; "$OURS/ls" -R rr | grep -c 'rr/l:')"
eq "ls -RL follows it" 1 "$("$OURS/ls" -RL rr | grep -c 'rr/l:')"
rm -rf rr
# a cycle
mkdir -p cy/a; ln -s .. cy/a/up
"$OURS/ls" -RL cy > o 2> e
eq "ls -RL in a directory tree with a link to a parent: a diagnostic, status 1, and it ends" "1 1" "$? $([ -s e ] && echo 1)"
"$OURS/ls" -R cy > o 2> e
eq "ls -R without -L does not follow it, and is fine" "0" "$?"
ln -s lb la; ln -s la lb
"$OURS/ls" -L la 2> e > /dev/null
eq "ls -L on a link loop: error" "1 1" "$? $([ -s e ] && echo 1)"
"$OURS/ls" -l la lb | wc -l | tr -d ' ' > o
eq "ls -l of the loop links themselves is fine" 2 "$(cat o)"
rm -rf cy la lb ld lnk flnk

# permissions that stop it
mkdir nd; : > nd/x; chmod 000 nd
"$OURS/ls" nd f1 2> e > /dev/null
eq "ls of an unreadable directory: a diagnostic, status 1, the other operand is listed" "1 1" "$("$OURS/ls" nd f1 > /dev/null 2>&1; echo $?) $(grep -c nd e)"
chmod 755 nd
mkdir -p rd/ok rd/bad; : > rd/ok/f; chmod 000 rd/bad
"$OURS/ls" -R rd 2> e > o
eq "ls -R with an unreadable subdirectory: the rest is listed, status 1" "1 1 1" "$("$OURS/ls" -R rd > /dev/null 2>&1; echo $?) $(grep -c 'rd/ok:' o) $(grep -c bad e)"
chmod 755 rd/bad; rm -rf nd rd

# a wide directory
mkdir wide
( cd wide && i=0 && while [ $i -lt 5000 ]; do : > "f$i"; i=$((i + 1)); done )
eq "ls of a directory with 5000 entries: all of them" 5000 "$("$OURS/ls" wide | wc -l | tr -d ' ')"
eq "ls wide is in the order of sort in the C locale" "$(LC_ALL=C "$OURS/ls" wide | LC_ALL=C sort | md5)" "$(LC_ALL=C "$OURS/ls" wide | md5)" 2> /dev/null
eq "ls -lS wide has the total and 5000 entries" 5001 "$("$OURS/ls" -lS wide | wc -l | tr -d ' ')"
rm -rf wide

# descriptors and signals
"$OURS/ls" f1 >&- 2> e
eq "ls with stdout closed: status 1 and a diagnostic" "1 1" "$? $([ -s e ] && echo 1)"
eq "ls into a pipe that ends early does not hang or complain" "0" "$("$OURS/ls" -R /usr 2> /dev/null | head -1 > /dev/null; echo 0)"
"$OURS/ls" f1 f2 < /dev/null <&- > /dev/null 2>&1
eq "ls with stdin closed" 0 $?

# PCTS ls 69 (GA53): -L has to evaluate the file a link refers to; for a link inside a listed
# directory that leads nowhere that is an error: diagnostic, status 1, the entry is not listed,
# the others are (-i is the option that forces the evaluation in the PCTS, -R in difftest).
fresh
mkdir dl
: > dl/ok
ln -s nowhere dl/dang
"$OURS/ls" -RLi dl > o 2> e
st=$?
eq "ls -RLi: a dangling link in a directory: status 1, diagnostic, not listed, the rest is" "1 err 0 1" "$st $([ -s e ] && echo err) $(grep -c dang o) $(grep -c ok o)"
"$OURS/ls" -Li dl > o 2> e
eq "ls -Li dl: same without -R" "1 err 0" "$? $([ -s e ] && echo err) $(grep -c dang o)"
"$OURS/ls" -L dl > o 2> e
eq "ls -L dl: the names alone need no evaluation, the link is listed, status 0" "0 2 empty" "$? $(wc -l < o | tr -d ' ') $([ -s e ] || echo empty)"
"$OURS/ls" -Ri dl > o 2> e
eq "ls -Ri dl without -L: the link is listed as itself, status 0" "0 2" "$? $(wc -l < o | tr -d ' ' | awk '{print $1-1}')"

# -f turns on -a and ignores -A, in either order
fresh
: > a
: > .h
eq "ls -fA and ls -Af list . and .. like -f" "4 4" "$("$OURS/ls" -fA | wc -l | tr -d ' ') $("$OURS/ls" -Af | wc -l | tr -d ' ')"

# the alternate access method flag: '+' for a file with an ACL (macOS), rows without one are padded
fresh
: > plain
: > acl
if chmod +a "everyone deny delete" acl 2> /dev/null; then
	"$OURS/ls" -l acl > o
	eq "ls -l: a file with an ACL has the + flag right after the ten characters" "11 +" "$(awk '{print length($1), substr($1,11,1)}' o)"
	"$OURS/ls" -l acl plain > o
	eq "ls -l: the row without a flag is padded to line up with the one that has it" "-rw-r--r--+ 1 -rw-r--r-- 1" "$(awk '{printf "%s %s ", $1, $2}' o | sed 's/ $//')"
	eq "ls -l: both rows have the link count at the same column" "$(sed -n 1p o | awk '{print index($0, " 1 ")}')" "$(sed -n 2p o | awk '{print index($0, " 1 ")}')"
	"$OURS/ls" -l plain > o
	eq "ls -l: no flag anywhere, no padding" "10" "$(awk '{print length($1)}' o)"
	# the ACL denies delete: take it off, or the cleanup of this directory fails
	chmod -N acl
fi


#------------------------------------------------- xargs, ln, chmod, chown
fresh

# xargs: the exit status says what happened to the utility (EXIT STATUS, CONSEQUENCES OF ERRORS)
echo a | "$OURS/xargs" sh -c 'exit 0' 2> e
eq "xargs: every invocation succeeds: 0" "0 0" "$? $(wc -c < e | tr -d ' ')"
printf 'a b c\n' | "$OURS/xargs" -n 1 sh -c 'exit 3' 2> e
eq "xargs: an invocation exits 3 (all of them run): 123" "123" "$?"
printf 'a b c\n' | "$OURS/xargs" -n 1 sh -c 'echo "$1" >> ran; [ "$1" = b ] && exit 255; exit 0' x 2> e
eq "xargs: an invocation exits 255: xargs ends, 124, nothing after it runs" "124 a b" "$? $(tr '\n' ' ' < ran | sed 's/ $//')"
rm -f ran
printf 'a b c\n' | "$OURS/xargs" -n 1 sh -c 'echo "$1" >> ran; [ "$1" = b ] && kill -9 $$; exit 0' x 2> e
eq "xargs: an invocation dies of a signal: 125, nothing after it runs" "125 a b" "$? $(tr '\n' ' ' < ran | sed 's/ $//')"
rm -f ran
echo a | "$OURS/xargs" ./nosuchcmd 2> e
eq "xargs: utility not found: 127 and a diagnostic" "127 1" "$? $(wc -l < e | tr -d ' ')"
: > noexec
echo a | "$OURS/xargs" ./noexec 2> e
eq "xargs: utility found but not executable: 126" "126" "$?"
rm -f noexec
echo a | "$OURS/xargs" sh -c 'exit 127' 2> e
eq "xargs: a utility that itself exits 127 is an invocation that failed, not a missing utility" "123" "$?"
echo a | "$OURS/xargs" sh -c 'exit 126' 2> e
eq "xargs: and 126 the same" "123" "$?"

# the utility reads /dev/null, not the input of xargs
printf 'x y\nz\n' | "$OURS/xargs" -n 1 sh -c 'cat | wc -c | tr -d " "' > out 2> e
eq "xargs: the utility's standard input is empty" "0 0 0" "$(tr '\n' ' ' < out | sed 's/ $//')"

# -t: "the utility and its constructed argument list shall be written to standard error, as it will be invoked,
# prior to invocation": one line for each invocation, before it, the arguments kept apart
printf 'a b\n' | "$OURS/xargs" -t echo hi 2>&1 > /dev/null | cat > e
eq "xargs -t: the command line on standard error" "echo hi a b" "$(cat e)"
printf '"a b" it\\'"'"'s ""\n' | "$OURS/xargs" -t echo 2>&1 > /dev/null | cat > e
eq "xargs -t: an argument with a blank, a quote or nothing in it is quoted, so that the arguments stay apart" "echo 'a b' \"it's\" ''" "$(cat e)"
printf 'a\nb\n' | "$OURS/xargs" -t -n 1 sh -c 'echo "run $1" >&2' x 2>&1 > /dev/null | cat > e
eq "xargs -t: each line is written just before its invocation" "sh -c 'echo \"run \$1\" >&2' x a|run a|sh -c 'echo \"run \$1\" >&2' x b|run b" "$(tr '\n' '|' < e | sed 's/|$//')"
printf 'a\n' | "$OURS/xargs" -t true > out 2> e
eq "xargs -t: nothing on standard output, one line on standard error" "0 true a" "$(wc -c < out | tr -d ' ') $(cat e)"

# size: every invocation is within the limit, nothing is lost, the order is kept
awk 'BEGIN { for (i = 1; i <= 200000; i++) printf "arg%07d\n", i }' > big
"$OURS/xargs" sh -c 'echo $#; for a; do echo "$a" >&2; done' x < big > counts 2> seen
eq "xargs: 200000 arguments: all passed, in order" "$(cat big | cksum)" "$(cksum < seen)"
eq "xargs: 200000 arguments are split over more than one command line" "yes" "$([ "$(wc -l < counts | tr -d ' ')" -gt 1 ] && echo yes)"
eq "xargs: and the arguments counted add up" "200000" "$(awk '{ n += $1 } END { print n }' counts)"
"$OURS/xargs" -n 7 sh -c 'echo $#' x < big > counts
eq "xargs -n 7: 28571 commands of 7 and one of 3" "28572 28571 3" "$(wc -l < counts | tr -d ' ') $(grep -c '^7$' counts) $(tail -1 counts)"
awk 'BEGIN { for (i = 0; i < 200000; i++) printf "x" ; print "" }' > huge
"$OURS/xargs" sh -c 'echo ran' x < huge > out 2> e
eq "xargs: an argument too big for any command line: 1, nothing run, a diagnostic" "1 0 1" "$? $(wc -c < out | tr -d ' ') $(wc -l < e | tr -d ' ')"
printf 'a\n%s\nb\n' "$(cat huge)" | "$OURS/xargs" -n 1 sh -c 'echo "$1" | wc -c' x > out 2> e
eq "xargs -n 1: the arguments before the one too big are run, then it ends" "1 1" "$? $(wc -l < out | tr -d ' ')"
rm -f big huge counts seen
echo a | "$OURS/xargs" -s 3 echo > out 2> e
eq "xargs -s smaller than the utility alone: 1, nothing run" "1 0" "$? $(wc -c < out | tr -d ' ')"

# the input
"$OURS/xargs" echo < /dev/null > out
eq "xargs: no input: the utility runs once" "0 1" "$? $(wc -l < out | tr -d ' ')"
"$OURS/xargs" echo x <&- > out 2> e
eq "xargs with standard input closed: an error, nothing run" "1 0" "$? $(wc -c < out | tr -d ' ')"
printf 'a \\\nb' | "$OURS/xargs" -n 1 printf '[%s]' > out
eq "xargs: an escaped newline is part of the argument" "[a][
b]" "$(cat out)"
printf 'a b' | "$OURS/xargs" -I {} echo '<{}>' > out
eq "xargs -I: a last line without a newline" "<a b>" "$(cat out)"
printf 'a\nb\n' | "$OURS/xargs" -t -n 1 echo > out 2> e
eq "xargs -t: the command lines on standard error, the output on standard output" "echo a
echo b a b" "$(cat e) $(tr '\n' ' ' < out | sed 's/ $//')"
printf 'a\0b\n' | "$OURS/xargs" echo > out 2> e
eq "xargs: a NUL byte in the input is an error, nothing run" "1 0" "$? $(wc -c < out | tr -d ' ')"

# -p reads the answers from the terminal, not from the input
if [ -x /usr/bin/script ] && /usr/bin/script -q /dev/null true < /dev/null > /dev/null 2>&1; then
	# script sends ^D when its input ends, so the answers are followed by a pause
	(sleep 1; printf 'y\nn\ny\n'; sleep 2) | /usr/bin/script -q /dev/null sh -c 'printf "a\nb\nc\n" | "$OURS/xargs" -p -n 1 echo > out' > ty 2>&1
	eq "xargs -p: y runs, n skips" "a c" "$(tr '\n' ' ' < out | sed 's/ $//')"
	eq "xargs -p: a prompt for each command line (?... after the -t line)" "3" "$(tr -d '\r' < ty | grep -o 'echo [abc]?\.\.\.' | wc -l | tr -d ' ')"
fi

# -x and the other size options
printf 'aaaa bbbb cccc\n' | "$OURS/xargs" -n 3 -s 13 -x echo > out 2> e
eq "xargs -n 3 -s 13 -x: the three do not fit: 1 and nothing run" "1 0" "$? $(wc -c < out | tr -d ' ')"
printf 'aaaa bbbb cccc\n' | "$OURS/xargs" -n 3 -s 13 echo > out 2> e
eq "xargs -n 3 -s 13: cut short without -x (echo and one argument are 10 bytes, two are 15)" "0 3" "$? $(wc -l < out | tr -d ' ')"

# a utility name or argument that is not text
printf 'a\n' | "$OURS/xargs" printf '%s\n' "$(printf '\377x')" > out
eq "xargs: bytes are passed through" "ff780a 610a" "$(head -1 out | od -An -tx1 | tr -d ' \n') $(tail -1 out | od -An -tx1 | tr -d ' \n')"

# ln
fresh
echo F > f
ln -s f sl
"$OURS/ln" -f f f 2> e
eq "ln -f f f: refused, f is kept" "1 F" "$? $(cat f)"
"$OURS/ln" -f sl f 2> e
eq "ln -f link-to-f f: refused, f is kept (its only name)" "1 F" "$? $(cat f)"
"$OURS/ln" f g
"$OURS/ln" -f sl g 2> e
eq "ln -f sl g: g is replaced, f keeps two names" "0 F 2" "$? $(cat g) $(ls -l f | awk '{print $2}')"
"$OURS/ln" nosuch h 2> e
eq "ln nosuch h: the diagnostic names the source" "1 1" "$? $(grep -c nosuch e)"
echo H > h
"$OURS/ln" -f nosuch h 2> e
eq "ln -f nosuch h: h is not removed for a link that cannot be made" "1 H" "$? $(cat h)"
mkdir dd; : > dd/x
"$OURS/ln" -f f dd/x
"$OURS/ln" -s f dd 2> e
eq "ln -s f dd: into the directory, as dd/f" "0 f" "$? $(readlink dd/f)"
"$OURS/ln" f g h dd 2> e
eq "ln f g h dd: dd/f exists: one error, the other two are linked" "1 1" "$? $(wc -l < e | tr -d ' ')"
"$OURS/ln" <&- >&- f nl 2>&-
eq "ln with the three standard descriptors closed" "0 F" "$? $(cat nl)"
"$OURS/ln" f 2> /dev/null
eq "ln f: one operand is a usage error" "2" "$?"

# chmod
fresh
echo x > f; chmod 644 f
"$OURS/chmod" 600 f <&- >&- 2>&-
eq "chmod with the standard descriptors closed" "0 -rw-------" "$? $(mode f)"
"$OURS/chmod" 600 nosuch 2>&-
eq "chmod of a missing file with standard error closed: 1" "1" "$?"
(umask 027; "$OURS/chmod" +rwx f)
eq "chmod +rwx under umask 027: the bits of the mask are left out" "-rwxr-x---" "$(mode f)"
(umask 077; "$OURS/chmod" a+r f)
eq "chmod a+r under umask 077: with a who the mask is not used" "-rwxr-xr--" "$(mode f)"
"$OURS/chmod" -R 755 nosuch f 2> e
eq "chmod -R 755 nosuch f: the error does not stop the next" "1 -rwxr-xr-x" "$? $(mode f)"
"$OURS/chmod" -- -w f 2> e
eq "chmod -- -w f: a mode that starts with -" "0 -r-xr-xr-x" "$? $(mode f)"
"$OURS/chmod" -w f 2> e
eq "chmod -w f: an option, so a usage error" "2" "$?"
chmod 644 f
mkdir -p t/a/b; echo 1 > t/top; echo 2 > t/a/b/f; ln -s ../top t/a/lnk; ln -s nowhere t/dang
chmod 600 t/top
"$OURS/chmod" -R go+r t 2> e
eq "chmod -R: status 0 and silent, a link and a dangling link are left alone" "0 0" "$? $(wc -c < e | tr -d ' ')"
eq "chmod -R: files and directories changed" "-rw-r--r-- drwxr-xr-x -rw-r--r--" "$(mode t/top) $(mode t/a) $(mode t/a/b/f)"
chmod 600 t/top
"$OURS/chmod" -R a-x t 2> e
eq "chmod -R a-x t: the directories are changed after what is in them: no error" "0 0" "$? $(wc -c < e | tr -d ' ')"
chmod -R u+rwx t
if [ "$(id -u)" -ne 0 ]; then
	mkdir shut; echo 1 > shut/f; chmod 000 shut
	"$OURS/chmod" -R u+rwx shut 2> e
	eq "chmod -R u+rwx on a directory that cannot be read: changed first, then read" "0 -rwxr--r--" "$? $(mode shut/f)"
	rm -rf shut
fi
# depth: a tree deeper than any pathname, a small stack, and few descriptors
mkdir deep
deeptree deep 300 1
(ulimit -s 512 2> /dev/null; "$OURS/chmod" -R 700 deep 2> e; echo $? > st)
eq "chmod -R of a tree 300 levels deep (over 5000 bytes of path), small stack" "0 0" "$(cat st) $(wc -c < e | tr -d ' ')"
(ulimit -n 12 2> /dev/null && "$OURS/chmod" -R 755 deep 2> e; echo $? > st)
eq "chmod -R of 300 levels with 12 descriptors allowed: every file changed" "0 0 0" "$(cat st) $(wc -c < e | tr -d ' ') $(find deep ! -perm 755 | wc -l | tr -d ' ')"
rm -rf deep

# chown: only what a user may do, to oneself
fresh
echo x > f; ln -s f sl; mkdir -p t/a; : > t/a/g; ln -s a t/up; ln -s nowhere t/dang
me=$(id -u); gr=$(id -g)
"$OURS/chown" "$me:$gr" f <&- >&- 2>&-
eq "chown with the standard descriptors closed" "0" "$?"
"$OURS/chown" "$me" nosuch 2>&-
eq "chown of a missing file with standard error closed: 1" "1" "$?"
"$OURS/chown" "$me" f nosuch f 2> e
eq "chown: an error does not stop the next file, one diagnostic" "1 1" "$? $(wc -l < e | tr -d ' ')"
"$OURS/chown" nosuch-user-poutil f 2> e
eq "chown: a user that does not exist: 1, a diagnostic" "1 1" "$? $(wc -l < e | tr -d ' ')"
"$OURS/chown" "$me:nosuch-group-poutil" f 2> e
eq "chown: a group that does not exist: 1" "1" "$?"
"$OURS/chown" -R "$me:$gr" t 2> e
eq "chown -R: the dangling link is a link itself and fine" "0 0" "$? $(wc -c < e | tr -d ' ')"
"$OURS/chown" -RL "$me:$gr" t 2> e
eq "chown -RL: the dangling link cannot be followed: one error" "1 1" "$? $(wc -l < e | tr -d ' ')"
"$OURS/chown" -h "$me:$gr" sl t/dang 2> /dev/null
eq "chown -h: of a link and of a dangling link" "0" "$?"
"$OURS/chown" "$me:$gr" sl
eq "chown of a link: the file it leads to" "0" "$?"
mkdir deep
deeptree deep 300 1
(ulimit -s 512 2> /dev/null; "$OURS/chown" -R "$me" deep 2> e; echo $? > st)
eq "chown -R of a tree 300 levels deep, small stack" "0 0" "$(cat st) $(wc -c < e | tr -d ' ')"
(ulimit -n 12 2> /dev/null && "$OURS/chown" -R "$me" deep 2> e; echo $? > st)
eq "chown -R of 300 levels with 12 descriptors allowed" "0 0" "$(cat st) $(wc -c < e | tr -d ' ')"
rm -rf deep

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

#------------------------------------------- closed standard descriptors
# A file opened later must not take the number of a closed 0, 1 or 2.
fresh
printf 'one\n' > a
"$OURS/cmp" a - <&- 2> /dev/null
eq "cmp a - with stdin closed: an error, not a comparison" 2 $?
"$OURS/paste" a - <&- > out 2> /dev/null
eq "paste a - with stdin closed: status 1" 1 $?
echo hi | "$OURS/tee" t /nonexistent/x > /dev/null 2>&-
eq "tee with stderr closed: no diagnostic in the output file" hi "$(cat t)"
echo hi | "$OURS/tee" t >&- 2> /dev/null
eq "tee with stdout closed: status 1, the file written once" "1 hi" "$? $(cat t)"
"$OURS/env" "$OURS/nohup" "$OURS/echo" hi >&- 2> /dev/null
eq "env: the utility it runs still finds stdout closed (nohup.out made)" hi "$(cat nohup.out 2> /dev/null)"

#------------------------------------------- SIGCHLD inherited as ignored
# Linux keeps an ignored SIGCHLD across exec and then wait() fails (macOS resets it at exec).
fresh
perl -e '$SIG{CHLD} = "IGNORE"; exec @ARGV' "$OURS/xargs" false < /dev/null 2> /dev/null
eq "xargs under an ignored SIGCHLD: the utility's failure is seen" 123 $?
perl -e '$SIG{CHLD} = "IGNORE"; exec @ARGV' "$OURS/time" false 2> /dev/null
eq "time under an ignored SIGCHLD: the utility's status" 1 $?

echo "regress: $((pass + fail)) checks, $fail failed"
[ "$fail" -eq 0 ]
