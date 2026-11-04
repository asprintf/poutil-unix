#!/bin/sh
# fuzz.sh - compare our utilities with the references on random input.
#
# Usage: sh difftest/fuzz.sh [-n COUNT] util...
#
# Each round makes random files (random bytes, or a few letters and newlines,
# sizes biased to the edges of the buffers), runs the utility and every
# reference that exists (/usr/bin, GNU g-prefixed with POSIXLY_CORRECT=1,
# and posixutils-rs when POSIXUTILS_RS is set) on the same arguments, and
# compares standard output and the exit status (zero / non-zero). The first
# difference is kept in a directory that is named, with the command line, so
# that it can be repeated. Unlike difftest.sh the input is random; the
# cases there are the fixed regression set. Differences that the standard
# leaves open are dropped by FUZZ_SKIP_<util> (see the fuzz_<util> functions).
#
# Environment: OURS (default target/release), LOCALES (default "C en_US.UTF-8"),
# POSIXUTILS_RS (default unset), FUZZ_KEEP (directory for failures, default
# $TMPDIR/fuzz-fail)

here=$(cd "$(dirname "$0")" && pwd)
OURS=${OURS:-$here/../target/release}
LOCALES=${LOCALES:-"C en_US.UTF-8"}
RS=${POSIXUTILS_RS:-}
KEEP=${FUZZ_KEEP:-${TMPDIR:-/tmp}/fuzz-fail}
count=200
[ "$1" = "-n" ] && { count=$2; shift 2; }
[ $# -ge 1 ] || { echo "usage: sh $0 [-n COUNT] util..." >&2; exit 2; }

tmp=$(mktemp -d "${TMPDIR:-/tmp}/fuzz.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT

# rnd N: a random number 0..N-1
rnd() { od -An -tu4 -N4 /dev/urandom | awk -v n="$1" 'NF { print $1 % n; exit }'; }

# size: a size biased to edges of buffers and word sizes
size() {
	case $(rnd 6) in
	0) rnd 20 ;;
	1) echo $((65536 + $(rnd 5) - 2)) ;;
	2) echo $((262144 + $(rnd 5) - 2)) ;;
	3) echo $((256 + $(rnd 5) - 2)) ;;
	4) rnd 5000 ;;
	5) rnd 400000 ;;
	esac
}

# take N: the first N bytes of standard input (head -c refuses 0 on macOS)
take() { if [ "$1" -eq 0 ]; then :; else head -c "$1"; fi; }

# mkfile NAME: random content, binary or a few letters and newlines
mkfile() {
	n=$(size)
	case $(rnd 3) in
	0) take "$n" < /dev/urandom > "$1" ;;
	1) take "$n" < /dev/urandom | LC_ALL=C tr '\000-\377' '[a*85][b*85][\n*86]' > "$1" ;;
	2) take "$n" < /dev/urandom | LC_ALL=C tr '\000-\377' '[a*40][b*40][c*40][ *30][\t*20][\n*30][d*56]' > "$1" ;;
	esac
}


# mktext NAME [MINIMUM]: ASCII text, a few letters, blanks, tabs, newlines, some
# backspaces and returns; it ends with a newline unless FUZZ_NONL is set
mktext() {
	n=$(size)
	[ "$n" -lt "${2:-0}" ] && n=${2:-0}
	take "$n" < /dev/urandom | LC_ALL=C tr '\000-\377' '[a*70][b*40][ *50][\t*20][\n*40][\b*10][\r*10][c*16]' > "$1"
	if [ -z "$FUZZ_NONL" ] && [ -s "$1" ] && [ "$(tail -c 1 "$1" | od -An -tx1 | tr -d ' ')" != 0a ]; then
		echo >> "$1"
	fi
	[ -s "$1" ] || echo x > "$1"
}

# mkutf8 NAME: lines of ASCII, accented letters, wide characters and blanks
mkutf8() {
	awk -v seed="$(rnd 1000000)" -v n="$(rnd 400)" 'BEGIN {
		srand(seed)
		split("a b c   \t \303\251 \342\202\254 \344\270\255 \303\274 x yz", t, " ")
		t[4] = " "; t[5] = "\t"
		for (i = 0; i < n; i++) {
			if (rand() < 0.08) { printf "\n"; continue }
			printf "%s", t[1 + int(rand() * 11)]
		}
		printf "\n"
	}' > "$1"
}

# check ARGS...: run ours and the references on the same arguments in $tmp/w
# (inputs: $STDIN, if set, is the file for standard input)
check() {
	for loc in ${CHECK_LOCALES:-$LOCALES}; do
		ref=
		(cd "$tmp/w" && env -i PATH=/usr/bin:/bin LC_ALL="$loc" POSIXLY_CORRECT=1 \
			"$OURS/$util" "$@" < "${STDIN:-/dev/null}" > "$tmp/o.ours" 2>/dev/null; echo $? > "$tmp/s.ours")
		for bin in /usr/bin/$util /bin/$util /opt/local/bin/g$util /opt/local/bin/gnu$util ${RS:+${NO_RS:-$RS/$util}}; do
			[ -x "$bin" ] || continue
			case $bin in /usr/bin/* | /bin/*) [ -n "$NO_MAC" ] && continue ;; esac
			(cd "$tmp/w" && env -i PATH=/usr/bin:/bin LC_ALL="$loc" POSIXLY_CORRECT=1 \
				"$bin" "$@" < "${STDIN:-/dev/null}" > "$tmp/o.ref" 2>/dev/null; echo $? > "$tmp/s.ref")
			[ "$(cat "$tmp/s.ours")" -eq 0 ] && so=0 || so=1
			[ "$(cat "$tmp/s.ref")" -eq 0 ] && sr=0 || sr=1
			if ! cmp -s "$tmp/o.ours" "$tmp/o.ref" || [ $so -ne $sr ]; then
				mkdir -p "$KEEP"
				d=$KEEP/$util.$$.$round
				rm -rf "$d"; cp -R "$tmp/w" "$d"
				echo "FUZZ DIFF $util [$loc] vs $bin: $*  (inputs kept in $d)"
				return 1
			fi
		done
	done
	return 0
}

fuzz_cksum() {
	mkfile a; mkfile b; : > empty
	[ -f a ] && [ -f b ] || { echo "fuzz: could not make the input files" >&2; exit 1; }
	check a || return
	check a b empty || return
	STDIN=a check || return
	STDIN=b check || return
	STDIN=a check a || return
	check b nosuch a || return
}

fuzz_fold() {
	mktext a; mktext b
	w=$((1 + $(rnd 40)))
	o=
	[ "$(rnd 2)" -eq 0 ] && o="$o -s"
	[ "$(rnd 4)" -eq 0 ] && o="$o -b"
	check $o -w $w a || return
	check $o -w $w a b || return
	check $o a || return
	STDIN=b check $o -w $w || return
	mkutf8 u
	CHECK_LOCALES=en_US.UTF-8 check $o -w $w u || return
}

fuzz_paste() {
	mktext a; mktext b; mktext c
	pool=', : | - \t \n \0 \\ x'
	list=
	i=$(rnd 4)
	while [ $i -ge 0 ]; do
		set -- $pool
		shift $(rnd 9)
		list="$list$1"
		i=$((i - 1))
	done
	case $(rnd 2) in 0) sf=;; 1) sf=-s;; esac
	o=$sf
	[ "$(rnd 2)" -eq 0 ] && o="$o -d $list"
	check $o a b || return
	check $o a b c || return
	check $o a || return
	STDIN=c check $o - a || return
	# macOS ends the lines of a second "-" with -s differently (see paste.known)
	case $o in
	*-s*) ;;
	*)
		STDIN=c check $o a - - || return
		STDIN=c check $o - - - || return
		;;
	esac
	mkutf8 u; mkutf8 v
	CHECK_LOCALES=en_US.UTF-8 check $sf -d "$(printf '\303\251\342\202\254')" u v || return
}

fuzz_id() {
	m=; n=; r=; u=
	case $(rnd 4) in 0) m=;; 1) m=-G;; 2) m=-g;; 3) m=-u;; esac
	[ "$(rnd 2)" -eq 0 ] && n=-n
	[ -n "$m" ] && [ "$m" != -G ] && [ "$(rnd 2)" -eq 0 ] && r=-r
	case $(rnd 7) in 0) u=;; 1) u=;; 2) u=root;; 3) u=nobody;; 4) u=daemon;; 5) u=nosuch;; 6) u=$(id -un);; esac
	if [ -z "$m" ]; then n=; r=; fi
	if [ -n "$u" ]; then
		# posixutils-rs reads the group file: other groups for a named user
		NO_RS=x check $m $n $r $u
	else
		check $m $n $r
	fi
}

fuzz_cut() {
	mktext a; mktext b; mkutf8 u
	pool='1 2 3 5 8 1-3 2-5 -4 7- 10-12 3-3 20- 4,2 1,1'
	list=
	i=$(rnd 3)
	while [ $i -ge 0 ]; do
		set -- $pool
		shift $(rnd 14)
		list="$list,$1"
		i=$((i - 1))
	done
	list=${list#,}
	check -b "$list" a || return
	check -c "$list" a b || return
	STDIN=b check -c "$list" || return
	set -- : ' ' a '	'
	shift $(rnd 4)
	d=$1
	[ "$d" = '\t' ] && d=$(printf '\t')
	if [ "$(rnd 2)" -eq 0 ]; then
		check -f "$list" -d "$d" a || return
	else
		check -f "$list" -d "$d" -s a b || return
	fi
	CHECK_LOCALES=en_US.UTF-8 check -c "$list" u || return
	CHECK_LOCALES=en_US.UTF-8 check -f "$list" -d "$(printf '\303\251')" u || return
}

# xargs: random input made of words, blanks, newlines, quotes, backslashes and the logical end-of-file
# string, with a random choice of -n, -L, -s, -x, -E and -I. The utility prints how many arguments it
# got and which. Left out: posixutils-rs (quoting), -s (they allow a command line of size bytes, we
# less than size), unmatched quotes (GNU runs what it read before) and an escaped blank ending a line
# (they take it for a continuation under -L). macOS squeezes the blanks of -I lines: GNU only there.
mkxargs_in() {
	awk -v seed="$(rnd 1000000)" -v n="$((1 + $(rnd 60)))" -v nq="$1" 'BEGIN {
		srand(seed)
		split("a bb ccc d_e _ EOF x1 y2", w, " ")
		for (i = 0; i < n; i++) {
			r = rand()
			if (r < 0.40) printf "%s", w[1 + int(rand() * 9)]
			else if (r < 0.55) printf " "
			else if (r < 0.62) printf "\t"
			else if (r < 0.75) printf "\n"
			else if (r < 0.80) printf "\\\\"
			else if (r < 0.85) printf " \\ x"
			else if (nq == 0) printf "x"
			else if (r < 0.90) printf "\"%s\"", w[1 + int(rand() * 9)]
			else if (r < 0.95) printf "\047%s\047", w[1 + int(rand() * 9)]
			else if (r < 0.97) printf "\"a b\""
			else printf "x"
		}
		if (rand() < 0.8) printf "\n"
	}' > "$2"
}

fuzz_xargs() {
	mkxargs_in $(rnd 2) in
	set -- sh -c 'echo "$#:$*"' x
	NO_RS=x
	case $(rnd 8) in
	0) o= ;;
	1) o="-n $((1 + $(rnd 5)))" ;;
	2) o="-L $((1 + $(rnd 4)))" ;;
	3) o="-n $((1 + $(rnd 9)))" ;;
	4) o="-L $((1 + $(rnd 9)))" ;;
	5) o="-E EOF" ;;
	6) o="-E _ -n 2" ;;
	7) o="-E '' -L 2" ;;
	esac
	case $(rnd 6) in 0) o="$o -x" ;; esac
	eval "STDIN=in NO_RS=x check $o \"\$@\"" || return
	STDIN=in NO_RS=x NO_MAC=x check -I '{}' echo '[{}]' || return
	STDIN=in NO_RS=x NO_MAC=x check -I '{}' -E EOF echo '<{}>' '{}' || return
}

# chmod: random symbolic and octal modes applied to files of every mode; what is compared is the mode
# of each file afterwards. macOS is left out (it keeps the set-ID and sticky bits that a mode with "="
# must clear and ignores "X" after "-"; see chmod.known.macos), and so are the modes of directories,
# where the set-ID and sticky bits of an octal mode are kept by GNU and posixutils-rs.
mkmode() {
	awk -v seed="$(rnd 1000000)" 'BEGIN {
		srand(seed)
		if (rand() < 0.12) {
			n = 1 + int(rand() * 5)
			for (i = 0; i < n; i++) printf "%d", int(rand() * 8)
			exit
		}
		nc = 1 + int(rand() * 3)
		for (c = 0; c < nc; c++) {
			if (c) printf ","
			nw = int(rand() * 4)
			for (i = 0; i < nw; i++) printf "%s", substr("ugoa", 1 + int(rand() * 4), 1)
			na = 1 + int(rand() * 3)
			for (a = 0; a < na; a++) {
				printf "%s", substr("+-=", 1 + int(rand() * 3), 1)
				if (rand() < 0.2) printf "%s", substr("ugo", 1 + int(rand() * 3), 1)
				else {
					np = int(rand() * 5)
					for (i = 0; i < np; i++) printf "%s", substr("rwxXst", 1 + int(rand() * 6), 1)
				}
			}
		}
	}'
}

fuzz_chmod() {
	m=$(mkmode)
	ops="f0 f4 f6 f7 f77 fs fg"
	for bin in /opt/local/bin/gchmod ${RS:+$RS/chmod}; do
		[ -x "$bin" ] || continue
		for who in ours ref; do
			rm -rf "$tmp/m"; mkdir "$tmp/m"
			(cd "$tmp/m" && touch f0 f4 f6 f7 f77 fs fg && chgrp "$(id -g)" $ops \
				&& chmod 000 f0 && chmod 400 f4 && chmod 644 f6 && chmod 755 f7 && chmod 777 f77 \
				&& chmod 4755 fs && chmod 2755 fg)
			if [ $who = ours ]; then b=$OURS/chmod; else b=$bin; fi
			(cd "$tmp/m" && env -i PATH=/usr/bin:/bin LC_ALL=C POSIXLY_CORRECT=1 "$b" "$m" $ops > /dev/null 2>&1; echo $? > "$tmp/s.$who"
				for f in $ops; do ls -ld "$f" | cut -c1-10; done > "$tmp/o.$who")
		done
		[ "$(cat "$tmp/s.ours")" -eq 0 ] && so=0 || so=1
		[ "$(cat "$tmp/s.ref")" -eq 0 ] && sr=0 || sr=1
		if ! cmp -s "$tmp/o.ours" "$tmp/o.ref" || [ $so -ne $sr ]; then
			echo "FUZZ DIFF chmod vs $bin: mode $m (status $so/$sr)"
			paste "$tmp/o.ours" "$tmp/o.ref" | tr '\t' ' ' | tr '\n' ';'; echo
			return 1
		fi
	done
	return 0
}

bad=0 total=0
for util in "$@"; do
	type "fuzz_$util" > /dev/null 2>&1 || { echo "$util: no fuzzer" >&2; continue; }
	[ -x "$OURS/$util" ] || { echo "$util: not built" >&2; continue; }
	round=0
	while [ $round -lt "$count" ]; do
		round=$((round + 1))
		rm -rf "$tmp/w"; mkdir "$tmp/w"
		cd "$tmp/w" || exit 1
		if ! "fuzz_$util"; then bad=$((bad + 1)); fi
		cd "$tmp" || exit 1
		total=$((total + 1))
	done
done
echo "fuzz: $total rounds, $bad with differences"
[ "$bad" -eq 0 ]
