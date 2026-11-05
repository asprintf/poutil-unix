# What the difftest work found and changed

Utilities covered: test, [, cp, xargs, wc, cat, touch, df, expr, chmod, chown.
Run: `sh difftest/difftest.sh UTIL...` (see the header of difftest/difftest.sh).
The Rust unit tests were kept: difftest and the unit tests complement each other.

## Bugs found in our utilities (all fixed, with unit tests)

xargs (POSIX issues):
- `-n` without `-s` let the first argument bypass the LINE_MAX limit; with `-x` it must stop.
- The default command size was a fixed 128 KiB; it must be at least LINE_MAX.
- `-t` printed arguments without quoting, so argument boundaries were lost (now quoted like GNU).

cp:
- `cp -P la a` with `la -> a` replaced `a` by a link to itself and destroyed the data. Now refused ("same file"), also for `-R`, `-H`, link chains and hard links of the target.
- A source ending in `..` (`cp -R d/sub/.. tgt`) wrote its files next to `tgt` instead of into it. Now treated like `.`.
- `cp -Rp` did not set mode, owner and times on a destination directory that already existed.
- `cp -f a loop` (destination is a symbolic link loop) failed; with `-f` it is now removed and replaced.

test:
- `test -t` with a non-integer operand now follows the spec (false, no error).

## Harness changes

- `errtext:` compares the text of standard error (without the program's own name), `umask:`, `in:`, `each:`, `SHARD=K/N`.
- Exact exit statuses for xargs (126/127), test and [ (1 false, >1 error), expr (1, 2, >2).
- A time limit per case (DIFFTEST_TIMEOUT, default 60 s, needs timeout or gtimeout).
- `difftest/coverage.sh` and `make difftest` / `make coverage` (coverage.sh was written but not run end to end).

## Differences that are accepted (listed in cases/UTIL.known*)

- cp -i, answer "no": GNU exits 1, ours exits 0 (the spec only calls errors nonzero).
- cp copying a directory into itself: GNU creates the destination and copies, then complains; ours refuses first. Both exit 1.
- cp: GNU warns when a source is named twice.
- Many chown/xargs/df/test lines where GNU or macOS differs from the spec; each has a comment.

## Not verified

- macOS (BSD) was not available here. New corpora and `.known.gnu` lines may need `.known.macos` twins.
- Runs were made on Linux against GNU coreutils/findutils as an unprivileged user in the C and C.utf8 locales.
- regress.sh: 26 checks failed on this Linux box at first. None was a bug in our utilities: the script expected macOS (uname from sysctl, 512-byte du and df, only en_US.UTF-8, md5, a PATH_MAX of 1024), mawk could not build huge strings, dash cannot cd into very deep trees, and ext4 cannot hold dates before 1901. regress.sh now finds a UTF-8 locale, uses the system uname, du and df in POSIX units, and skips the 1900 check where the file system cannot hold it. Result as an unprivileged user: 1418 checks, 0 failed. Skipped here: locales de_DE and fr_FR, and the checks that need script(1) (xargs -p and rm/ls on a terminal).
- xargs: `<blank>` in UTF-8 locales is treated as byte blanks only (same as GNU).
