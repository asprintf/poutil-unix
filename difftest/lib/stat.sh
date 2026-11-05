# stat.sh - file attributes in one spelling for BSD stat (macOS) and GNU stat
# (Linux). A UTIL.show script sources it: . "$DIFFTEST_LIB/stat.sh"
# Every function takes a path and prints the same text on both, so the output of
# a show script does not depend on the platform it was written on.
if stat -c %n / >/dev/null 2>&1; then
	st_mode() { stat -c %A "$1"; }
	st_ids() { stat -c '%u %g' "$1"; }
	st_size() { stat -c %s "$1"; }
	st_mtime() { stat -c %.9Y "$1"; }
	st_atime() { stat -c %.9X "$1"; }
	st_mtime_s() { stat -c %Y "$1"; }
	st_atime_s() { stat -c %X "$1"; }
	st_ino() { stat -c %i "$1"; }
	st_nlink() { stat -c %h "$1"; }
	# mode, size, modification and access time (seconds) in one call: show scripts that list many files need it
	st_all() { stat -c '%A %s %Y %X' "$1"; }
else
	st_mode() { stat -f %Sp "$1"; }
	st_ids() { stat -f '%u %g' "$1"; }
	st_size() { stat -f %z "$1"; }
	st_mtime() { stat -f %Fm "$1"; }
	st_atime() { stat -f %Fa "$1"; }
	st_mtime_s() { stat -f %m "$1"; }
	st_atime_s() { stat -f %a "$1"; }
	st_ino() { stat -f %i "$1"; }
	st_nlink() { stat -f %l "$1"; }
	st_all() { stat -f '%Sp %z %m %a' "$1"; }
fi
