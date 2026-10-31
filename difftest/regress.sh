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

#----------------------------------------------------- sleep tty logname
fresh
"$OURS/sleep" 0
eq "sleep 0" 0 $?
"$OURS/sleep" 1.5 2> /dev/null
eq "sleep 1.5: refused" 1 $?
"$OURS/sleep" 99999999999999999999999 < /dev/null > /dev/null 2>&1 &
p=$!
sleep 1
"$OURS/kill" -s 0 $p
eq "sleep with a huge time: still asleep after a second" 0 $?
"$OURS/kill" $p
reap $p
"$OURS/tty" < /dev/null > out
eq "tty on /dev/null: status" 1 $?
eq "tty on /dev/null: output" "not a tty" "$(cat out)"
"$OURS/tty" -s < /dev/null 2> /dev/null
eq "tty -s: not an option of the 2018 edition" 2 $?

#----------------------------------------------------------------------- wc
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

echo "regress: $((pass + fail)) checks, $fail failed"
[ "$fail" -eq 0 ]
