#!/bin/sh
# difftest.sh - compare our utilities with reference implementations.
#
# Usage: sh difftest/difftest.sh [-v] util...
#
# For each case line in difftest/cases/UTIL (shell words, # comments allowed) the
# utility is run in a fresh directory, prepared by difftest/cases/UTIL.setup if
# present, once for ours and once for each reference: the native one in
# /usr/bin (or /bin) and the GNU one (g-prefixed from MacPorts; gnucut where gcut would clash) with
# POSIXLY_CORRECT=1. stdout, whether stderr is empty, the exit status
# (zero/non-zero; exact for tty) and the resulting directory listing are
# compared, in the C locale and in a UTF-8 locale.
#
# Optional per-utility files next to the case file: UTIL.setup (run in the
# fresh directory first), UTIL.stdin (standard input instead of /dev/null),
# UTIL.show (run afterwards; its output is compared like the listing, for
# modes and file contents), UTIL.filter (a shell script that rewrites standard
# output before it is compared, for what the spec leaves open: the padding of
# wc, the order of tsort).
#
# A line "@" means no arguments. Case lines also listed in UTIL.known are
# reported as "known" (judged acceptable) and not counted as differences.
#
# A third reference is optional: if POSIXUTILS_RS names a directory holding
# built posixutils-rs binaries, UTIL from there is compared too, as a black
# box (it is only ever run, never read). Lines accepted only for that one are
# listed in UTIL.known.posixutils; UTIL.known applies to every reference.
#
# Environment: OURS (default target/release), LOCALES (default "C en_US.UTF-8"),
# POSIXUTILS_RS (default unset: no third reference), SHARD=K/N (run only every
# Nth case line, those with number mod N equal to K: start N copies with
# K = 0 .. N-1 at the same time to use all the processors)

here=$(cd "$(dirname "$0")" && pwd)
OURS=${OURS:-$here/../target/release}
LOCALES=${LOCALES:-"C en_US.UTF-8"}
RS=${POSIXUTILS_RS:-}
verbose=0
[ "$1" = "-v" ] && { verbose=1; shift; }
[ $# -ge 1 ] || { echo "usage: sh $0 [-v] util..." >&2; exit 2; }

tmp=$(mktemp -d "${TMPDIR:-/tmp}/difftest.XXXXXX") || exit 1
trap 'chmod -R u+rwx "$tmp" 2>/dev/null; rm -rf "$tmp"' EXIT

# run BIN LOCALE: runs "$@" in a fresh directory, leaves results in $tmp/r.*
run() {
	bin=$1 loc=$2
	shift 2
	work=$tmp/work
	chmod -R u+rwx "$work" 2>/dev/null
	rm -rf "$work"
	mkdir "$work"
	in=/dev/null
	[ -f "$here/cases/$util.stdin" ] && in=$here/cases/$util.stdin
	if [ -f "$here/cases/$util.setup" ]; then
		(cd "$work" && sh "$here/cases/$util.setup") >/dev/null 2>&1
	fi
	(cd "$work" && env -i PATH=/usr/bin:/bin PWD="$work" LC_ALL="$loc" \
		POSIXLY_CORRECT=1 "$bin" "$@") <"$in" >"$tmp/r.out" 2>"$tmp/r.err"
	echo $? >"$tmp/r.st"
	if [ -f "$here/cases/$util.filter" ]; then
		sh "$here/cases/$util.filter" <"$tmp/r.out" >"$tmp/r.flt" 2>/dev/null
		mv "$tmp/r.flt" "$tmp/r.out"
	fi
	(cd "$work" && ls -1) >"$tmp/r.fs" 2>&1
	if [ -f "$here/cases/$util.show" ]; then
		(cd "$work" && env -i PATH=/usr/bin:/bin sh "$here/cases/$util.show") >>"$tmp/r.fs" 2>&1
	fi
}

# norm STATUS: zero / non-zero, except where the value matters
norm() {
	case $util in tty | env | nice | nohup | time) echo "$1"; return ;; esac
	if [ "$util" = tty ]; then echo "$1"; elif [ "$1" -eq 0 ]; then echo 0; else echo nz; fi
}

total=0 bad=0 missing=0
for util in "$@"; do
	cases=$here/cases/$util
	[ -f "$cases" ] || { echo "$util: no case file" >&2; missing=$((missing + 1)); continue; }
	refs=
	for r in /usr/bin/$util /bin/$util /opt/local/bin/g$util /opt/local/bin/gnu$util; do
		[ -x "$r" ] && refs="$refs $r"
	done
	[ -n "$RS" ] && [ -x "$RS/$util" ] && refs="$refs $RS/$util"
	lineno=0
	while IFS= read -r line; do
		case $line in '' | '#'*) continue ;; esac
		lineno=$((lineno + 1))
		[ -z "$SHARD" ] || [ $((lineno % ${SHARD#*/})) -eq "${SHARD%/*}" ] || continue
		for loc in $LOCALES; do
			if [ "$line" = "@" ]; then set --; else eval "set -- $line"; fi
			run "$OURS/$util" "$loc" "$@"
			cp "$tmp/r.out" "$tmp/o.out"
			o_err=$([ -s "$tmp/r.err" ] && echo err || echo quiet)
			o_st=$(norm "$(cat "$tmp/r.st")")
			cp "$tmp/r.fs" "$tmp/o.fs"
			for ref in $refs; do
				total=$((total + 1))
				run "$ref" "$loc" "$@"
				r_err=$([ -s "$tmp/r.err" ] && echo err || echo quiet)
				r_st=$(norm "$(cat "$tmp/r.st")")
				why=
				cmp -s "$tmp/o.out" "$tmp/r.out" || why="$why stdout"
				[ "$o_err" = "$r_err" ] || why="$why stderr($o_err/$r_err)"
				[ "$o_st" = "$r_st" ] || why="$why status($o_st/$r_st)"
				cmp -s "$tmp/o.fs" "$tmp/r.fs" || why="$why files"
				name=$ref is_rs=0
				if [ -n "$RS" ] && [ "$ref" = "$RS/$util" ]; then
					name=posixutils-rs is_rs=1
				fi
				known=0
				grep -qxF -- "$line" "$cases.known" 2>/dev/null && known=1
				if [ $known -eq 0 ] && [ $is_rs -eq 1 ] &&
				    grep -qxF -- "$line" "$cases.known.posixutils" 2>/dev/null; then
					known=1
				fi
				if [ -n "$why" ] && [ $known -eq 1 ]; then
					echo "known $util [$loc] $line  vs $name:$why"
				elif [ -n "$why" ]; then
					bad=$((bad + 1))
					echo "DIFF $util [$loc] $line  vs $name:$why"
					if [ $verbose -eq 1 ]; then
						echo "  ours: $(od -c "$tmp/o.out" | head -3 | tr '\n' '|')"
						echo "  ref:  $(od -c "$tmp/r.out" | head -3 | tr '\n' '|')"
					fi
				fi
			done
		done
	done <"$cases"
done
echo "difftest: $total comparisons, $bad differences"
[ "$missing" -eq 0 ] || echo "difftest: $missing without a case file" >&2
[ "$bad" -eq 0 ] && [ "$missing" -eq 0 ]
