#!/bin/sh
# coverage.sh - which lines of the utilities do difftest.sh and regress.sh run?
#
# Usage: sh difftest/coverage.sh [util...]
#
# Builds the utilities with LLVM source-based coverage into target/cov, runs
# difftest.sh for the given utilities (default: every one that has a case file)
# with that build, then prints the coverage of each utility's source and writes
# an HTML report to target/cov/html (open target/cov/html/index.html).
# The unit tests of cargo test are not part of it: they are a different measure.
#
# Needs llvm-profdata and llvm-cov (rustup component add llvm-tools-preview, or the
# ones next to the compiler: the toolchain's lib/rustlib/*/bin, or in your PATH).
# Environment: LOCALES as for difftest.sh; JOBS shards for difftest.sh (default 3, at
# most 3); UNCOVERED=1 prints the source lines that no case ran; COVERAGE_REGRESS=1
# also runs regress.sh (the full one: never do that while implementing or fixing).

here=$(cd "$(dirname "$0")" && pwd)
root=$here/..
cov=$root/target/cov

for t in llvm-profdata llvm-cov; do
	command -v "$t" > /dev/null 2>&1 && continue
	found=$(ls "$HOME"/.rustup/toolchains/*/lib/rustlib/*/bin/"$t" "$(rustc --print sysroot 2> /dev/null)"/lib/rustlib/*/bin/"$t" 2> /dev/null | head -n 1)
	[ -n "$found" ] || { echo "coverage: $t not found" >&2; exit 1; }
	PATH=$(dirname "$found"):$PATH
done

if [ $# -eq 0 ]; then
	set --
	for f in "$here"/cases/*; do
		case $f in *.*) continue ;; esac
		set -- "$@" "$(basename "$f")"
	done
fi

(cd "$root" && RUSTFLAGS="-C instrument-coverage" cargo build --release --target-dir "$cov") || exit 1
ln -f "$cov/release/test" "$cov/release/[" 2> /dev/null

rm -rf "$cov/prof" "$cov/html"
mkdir "$cov/prof"
# one file for each process: %p is the process, %m the binary
LLVM_PROFILE_FILE="$cov/prof/%p-%m.profraw"
export LLVM_PROFILE_FILE
OURS=$cov/release sh "$here/difftest.sh" -j "${JOBS:-3}" "$@"
[ -z "${COVERAGE_REGRESS:-}" ] || OURS=$cov/release sh "$here/regress.sh"
unset LLVM_PROFILE_FILE

llvm-profdata merge -sparse "$cov"/prof/*.profraw -o "$cov/all.profdata" || exit 1
first=
objs=
for u in "$@"; do
	[ -x "$cov/release/$u" ] || continue
	if [ -z "$first" ]; then first=$cov/release/$u; else objs="$objs -object=$cov/release/$u"; fi
done
[ -n "$first" ] || { echo "coverage: no binary to report" >&2; exit 1; }
# only our own sources: not the standard library, not dependencies
skip='/\.cargo/|/rustc/|/rustlib/'
# shellcheck disable=SC2086
llvm-cov report "$first" $objs -instr-profile="$cov/all.profdata" --ignore-filename-regex="$skip"
# shellcheck disable=SC2086
llvm-cov show "$first" $objs -instr-profile="$cov/all.profdata" --ignore-filename-regex="$skip" \
	-format=html -output-dir="$cov/html" > /dev/null
echo "coverage: html report in $cov/html/index.html"
if [ -n "${UNCOVERED:-}" ]; then
	# the lines of the sources of the utilities asked for that ran 0 times
	for u in "$@"; do
		[ -x "$cov/release/$u" ] || continue
		# shellcheck disable=SC2086
		llvm-cov show "$cov/release/$u" -instr-profile="$cov/all.profdata" --ignore-filename-regex="$skip" \
			-sources "$root/$u/$u.rs" -format=text 2> /dev/null |
			awk -v u="$u" '$0 ~ /^ +[0-9]+\| +0\|/ { sub(/^ +/, ""); print u ".rs:" $0 }'
	done
fi
