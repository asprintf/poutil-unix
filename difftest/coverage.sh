#!/bin/sh
# coverage.sh - which lines of the utilities do difftest.sh and regress.sh run?
#
# Usage: sh difftest/coverage.sh [util...]
#
# Builds the utilities with LLVM source-based coverage into target/cov, runs
# difftest.sh for the given utilities (default: every one that has a case file)
# and regress.sh with that build, then prints the coverage of each utility's source
# and writes an HTML report to target/cov/html (open target/cov/html/index.html).
# The unit tests of cargo test are not part of it: they are a different measure.
#
# Needs llvm-profdata and llvm-cov (rustup component add llvm-tools-preview; the
# tools are then under the toolchain's lib/rustlib/*/bin, or in your PATH).
# Environment: LOCALES as for difftest.sh.

here=$(cd "$(dirname "$0")" && pwd)
root=$here/..
cov=$root/target/cov

for t in llvm-profdata llvm-cov; do
	command -v "$t" > /dev/null 2>&1 && continue
	found=$(ls "$HOME"/.rustup/toolchains/*/lib/rustlib/*/bin/"$t" 2> /dev/null | head -n 1)
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
OURS=$cov/release sh "$here/difftest.sh" "$@"
OURS=$cov/release sh "$here/regress.sh"
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
