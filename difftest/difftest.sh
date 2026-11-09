#!/bin/sh
# difftest.sh - compare our utilities with reference implementations.
#
# Usage: sh difftest/difftest.sh [-v] [-j N] util...   (-j N: N shards at once, at most 3; progress and the
# time of each utility go to standard error)
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
# First words on a case line that are no arguments (any number, in any order):
#   in:NAME    standard input from the file cases/UTIL.in/NAME for that line only (xargs)
#   umask:MASK the utility runs with that umask (cp, chmod: what the mode of a new file is)
#   path:DIRS  PATH for the utility is DIRS followed by /usr/bin:/bin (relative to the case directory; xargs: the search for the utility)
#   errtext:   the text of standard error is compared too, without the lines that start
#              with the name of the program ("xargs: ..." and "/usr/bin/xargs: ..." differ
#              and the standard leaves diagnostics open); for what the standard writes
#              there, such as the lines of xargs -t. Without it only "is there any" counts.
#
# UTIL.setup and UTIL.show get DIFFTEST_LIB, the directory lib/ (stat.sh: a stat that
# prints the same on macOS and on Linux). UTIL.setup runs with LC_ALL=C (bytes, not characters: BSD tr and sed refuse
# invalid UTF-8). CORPUS (environment of UTIL.setup and UTIL.show) names the directory
# cases/UTIL.in, so a setup file can copy corpus files into the fresh directory:
# cp "$CORPUS/emoji" .
#
# A line "each:NAME TEMPLATE" is a case line for every line of cases/UTIL.in/NAME
# (blank lines and # comments skipped): the first @@ in TEMPLATE is replaced by
# that line as one quoted word. "each:modes -- @@ f" runs chmod once per mode.
#
# A line "@" means no arguments. Case lines also listed in UTIL.known are
# reported as "known" (judged acceptable) and not counted as differences.
#
# UTIL.known applies to every reference; UTIL.known.gnu and UTIL.known.macos to one of them.
#
# ONLY_LINES=FILE runs only the case lines that appear in FILE (whole lines, as in the case file):
# one case, or the lines of a .known file, without the rest of the utility.
# Environment: DIFFTEST_TIMEOUT (seconds a case may run, default 60; needs timeout or gtimeout), OURS (default target/release), LOCALES (default "C en_US.UTF-8"),
# LLVM_PROFILE_FILE (passed on to ours, for difftest/coverage.sh),
# SHARD=K/N (run only every
# Nth case line, those with number mod N equal to K: start N copies with
# K = 0 .. N-1 at the same time to use all the processors)
#
# A case line is run by eval in this shell, so it can call othergroup: a group of the user other than the
# primary one (the primary one if there is none), "$(othergroup)": no group name is assumed.

othergroup() {
	id -G | tr ' ' '\n' | awk -v e="$(id -g)" 'BEGIN { o = e } $1 != e { o = $1; exit } END { print o }'
}

here=$(cd "$(dirname "$0")" && pwd)
OURS=${OURS:-$here/../target/release}
LOCALES=${LOCALES:-"C en_US.UTF-8"}
verbose=0 jobs=1
while :; do
	case ${1-} in
	-v) verbose=1; shift ;;
	-j) jobs=${2-}; shift 2 ;;
	-j*) jobs=${1#-j}; shift ;;
	*) break ;;
	esac
done
case $jobs in '' | *[!0-9]* | 0) jobs=1 ;; esac
[ $# -ge 1 ] || { echo "usage: sh $0 [-v] [-j N] util..." >&2; exit 2; }

# -j N: N copies of this script with SHARD=K/N for each utility in turn, their output joined (N is at most 3)
if [ "$jobs" -gt 1 ] && [ -z "${SHARD:-}" ]; then
	[ "$jobs" -le 3 ] || jobs=3
	vflag=
	[ $verbose -eq 0 ] || vflag=-v
	pdir=$(mktemp -d "${TMPDIR:-/tmp}/difftest.j.XXXXXX") || exit 1
	pids=
	trap 'kill $pids 2>/dev/null; rm -rf "$pdir"' EXIT
	trap 'exit 130' INT TERM
	total=0 bad=0 rc=0
	for u in "$@"; do
		pids=
		k=0
		while [ $k -lt $jobs ]; do
			SHARD=$k/$jobs sh "$here/difftest.sh" $vflag "$u" >"$pdir/out.$k" &
			pids="$pids $!"
			k=$((k + 1))
		done
		for p in $pids; do wait "$p" || rc=1; done
		pids=
		k=0
		while [ $k -lt $jobs ]; do
			grep -v '^difftest: [0-9]* comparisons' "$pdir/out.$k"
			k=$((k + 1))
		done
		eval "$(cat "$pdir"/out.* | awk -v t="$total" -v b="$bad" '/^difftest: [0-9]+ comparisons/ { t += $2; b += $4 } END { print "total=" t " bad=" b }')"
	done
	echo "difftest: $total comparisons, $bad differences"
	exit $rc
fi

# A case that does not end (cp -p on a FIFO nobody writes to is a read that waits for ever, an endless
# recursion) is killed after DIFFTEST_TIMEOUT seconds (default 60) if timeout(1) or gtimeout is there; the
# status is then 124, for ours and for the reference alike. Not for the utilities whose process
# surroundings are part of what is tested.
limit_cmd=
for t in timeout gtimeout; do
	p=$(command -v $t 2>/dev/null) && { limit_cmd="$p -k 5 ${DIFFTEST_TIMEOUT:-60}"; break; }
done

tmp=$(mktemp -d "${TMPDIR:-/tmp}/difftest.XXXXXX") || exit 1
trap 'chmod -R u+rwx "$tmp" 2>/dev/null; rm -rf "$tmp"' EXIT

# run BIN LOCALE: runs "$@" in a fresh directory, leaves results in $tmp/r.*
run() {
	bin=$1 loc=$2
	shift 2
	lim=$limit_cmd
	case $util in tty | env | nice | nohup | time | kill) lim= ;; esac
	work=$tmp/work
	chmod -R u+rwx "$work" 2>/dev/null
	rm -rf "$work"
	mkdir "$work"
	in=/dev/null
	[ -f "$here/cases/$util.stdin" ] && in=$here/cases/$util.stdin
	[ -n "$case_in" ] && in=$here/cases/$util.in/$case_in
	if [ -f "$here/cases/$util.setup" ]; then
		(cd "$work" && LC_ALL=C CORPUS=$here/cases/$util.in DIFFTEST_LIB=$here/lib sh "$here/cases/$util.setup") >/dev/null 2>&1
	fi
	if [ -n "$case_mask" ]; then
		(cd "$work" && env -i PATH="${case_path:+$case_path:}/usr/bin:/bin" PWD="$work" LC_ALL="$loc" \
			POSIXLY_CORRECT=1 ${LLVM_PROFILE_FILE:+LLVM_PROFILE_FILE="$LLVM_PROFILE_FILE"} \
			sh -c 'umask "$1" && shift && exec "$@"' sh "$case_mask" $lim "$bin" "$@") <"$in" >"$tmp/r.out" 2>"$tmp/r.err"
	else
		(cd "$work" && env -i PATH="${case_path:+$case_path:}/usr/bin:/bin" PWD="$work" LC_ALL="$loc" \
			POSIXLY_CORRECT=1 ${LLVM_PROFILE_FILE:+LLVM_PROFILE_FILE="$LLVM_PROFILE_FILE"} $lim "$bin" "$@") <"$in" >"$tmp/r.out" 2>"$tmp/r.err"
	fi
	echo $? >"$tmp/r.st"
	# the text of standard error without the program's own diagnostics, for errtext: lines
	grep -v -e "^$bin: " -e "^$(basename "$bin"): " "$tmp/r.err" >"$tmp/r.et" 2>/dev/null
	if [ -f "$here/cases/$util.filter" ]; then
		sh "$here/cases/$util.filter" <"$tmp/r.out" >"$tmp/r.flt" 2>/dev/null
		mv "$tmp/r.flt" "$tmp/r.out"
	fi
	(cd "$work" && ls -1) >"$tmp/r.fs" 2>&1
	if [ -f "$here/cases/$util.show" ]; then
		(cd "$work" && env -i PATH=/usr/bin:/bin CORPUS="$here/cases/$util.in" DIFFTEST_LIB="$here/lib" sh "$here/cases/$util.show") >>"$tmp/r.fs" 2>&1
	fi
}

# norm STATUS: zero / non-zero, except where the value matters
norm() {
	case $util in tty | env | nice | nohup | time) echo "$1"; return ;; esac
	# xargs: 126 and 127 say what went wrong with the utility, any other failure is 1-125 (EXIT STATUS)
	if [ "$util" = xargs ]; then case $1 in 0 | 126 | 127) echo "$1" ;; *) echo nz ;; esac; return; fi
	# test and [: 1 is false, above 1 an error. expr: 1 is a null or zero result, 2 an invalid expression,
	# above 2 an error (their EXIT STATUS sections)
	case $util in
	test | '[') case $1 in 0 | 1) echo "$1" ;; *) echo err ;; esac; return ;;
	expr) case $1 in 0 | 1 | 2) echo "$1" ;; *) echo ">2" ;; esac; return ;;
	esac
	if [ "$util" = tty ]; then echo "$1"; elif [ "$1" -eq 0 ]; then echo 0; else echo nz; fi
}

# the tools in /usr/bin are the GNU ones on Linux, the BSD ones elsewhere; that
# decides whether UTIL.known.gnu or UTIL.known.macos applies to them
case $(uname -s) in Linux) native=gnu ;; *) native=macos ;; esac

total=0 bad=0 missing=0
for util in "$@"; do
	cases=$here/cases/$util
	[ -f "$cases" ] || { echo "$util: no case file" >&2; missing=$((missing + 1)); continue; }
	refs=
	for r in /usr/bin/$util /bin/$util /usr/sbin/$util /opt/local/bin/g$util /opt/local/bin/gnu$util; do
		[ -x "$r" ] || continue
		# /bin and /usr/bin often hold the same file: one reference, not two
		dup=0
		for seen in $refs; do cmp -s "$r" "$seen" && dup=1; done
		[ $dup -eq 1 ] || refs="$refs $r"
	done
	# a line "each:NAME TEMPLATE" stands for one case line per line of
	# cases/UTIL.in/NAME, with the first @@ of TEMPLATE replaced by that line,
	# quoted as one word (a corpus of values: modes, times, expressions)
	expanded=$tmp/cases.expanded
	while IFS= read -r line; do
		case $line in
		each:*)
			rest=${line#each:}
			name=${rest%% *}
			tmpl=${rest#* }
			while IFS= read -r val || [ -n "$val" ]; do
				case $val in '' | '#'*) continue ;; esac
				# quoted only when it has to be: the line then reads as one written by hand
				case $val in
				*[!A-Za-z0-9_./:=+@%^,-]*) q="'$(printf '%s' "$val" | sed "s/'/'\\\\''/g")'" ;;
				*) q=$val ;;
				esac
				printf '%s\n' "${tmpl%%@@*}$q${tmpl#*@@}"
			done <"$here/cases/$util.in/$name"
			;;
		*) printf '%s\n' "$line" ;;
		esac
	done <"$cases" >"$expanded"
	lineno=0
	nlines=$(grep -v -e '^$' -e '^#' "$expanded" | wc -l | tr -d ' ')
	ustart=$(date +%s)
	while IFS= read -r line; do
		case $line in '' | '#'*) continue ;; esac
		lineno=$((lineno + 1))
		[ $((lineno % 50)) -ne 0 ] || echo "difftest: $util [${SHARD:-all}] case line $lineno of $nlines, $(($(date +%s) - ustart)) s, $bad differences so far" >&2
		[ -z "$SHARD" ] || [ $((lineno % ${SHARD#*/})) -eq "${SHARD%/*}" ] || continue
		[ -z "${ONLY_LINES:-}" ] || grep -qxF -- "$line" "$ONLY_LINES" || continue
		for loc in $LOCALES; do
			if [ "$line" = "@" ]; then set --; else eval "set -- $line"; fi
			# first words that are no arguments: in:NAME (standard input from cases/UTIL.in/NAME), umask:MASK, errtext:
			case_in= case_mask= case_path= case_et=0
			while :; do
				case ${1-} in
				in:*) case_in=${1#in:}; shift ;;
				umask:*) case_mask=${1#umask:}; shift ;;
				path:*) case_path=${1#path:}; shift ;;
				errtext:) case_et=1; shift ;;
				*) break ;;
				esac
			done
			run "$OURS/$util" "$loc" "$@"
			cp "$tmp/r.out" "$tmp/o.out"
			cp "$tmp/r.et" "$tmp/o.et"
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
				[ $case_et -eq 0 ] || cmp -s "$tmp/o.et" "$tmp/r.et" || why="$why stderr-text"
				name=$ref
				known=0
				grep -qxF -- "$line" "$cases.known" 2>/dev/null && known=1
				if [ $known -eq 0 ]; then
					case $ref in /opt/local/bin/*) kind=gnu ;; *) kind=$native ;; esac
					grep -qxF -- "$line" "$cases.known.$kind" 2>/dev/null && known=1
				fi
				if [ -n "$why" ] && [ $known -eq 1 ]; then
					printf '%s\n' "known $util [$loc] $line  vs $name:$why"
				elif [ -n "$why" ]; then
					bad=$((bad + 1))
					printf '%s\n' "DIFF $util [$loc] $line  vs $name:$why"
					if [ $verbose -eq 1 ]; then
						echo "  ours: $(od -c "$tmp/o.out" | head -3 | tr '\n' '|')"
						echo "  ref:  $(od -c "$tmp/r.out" | head -3 | tr '\n' '|')"
					fi
				fi
			done
		done
	done <"$expanded"
	echo "difftest: $util [${SHARD:-all}] done in $(($(date +%s) - ustart)) s" >&2
done
echo "difftest: $total comparisons, $bad differences"
[ "$missing" -eq 0 ] || echo "difftest: $missing without a case file" >&2
[ "$bad" -eq 0 ] && [ "$missing" -eq 0 ]
