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

# st_tree FORMAT [nodot]: one line per entry under . (nodot: without .), sorted, from one find+stat, one
# cksum and one awk: the same listing as a loop of st_* calls costs a process per file and per field.
# FORMAT: chmod (type, name, mode), chown (+ ids), cp (mode, size, times, sum), touch (times, size, mode)
st_tree() {
	if stat -c %n / >/dev/null 2>&1; then
		sf=-c sx='%A %u %g %s %.9Y %.9X %n'
	else
		sf=-f sx='%Sp %u %g %z %Fm %Fa %N'
	fi
	now=$(date +%s)
	{
		if [ "${2-}" = nodot ]; then
			find . ! -name . -exec stat "$sf" "$sx" {} + 2> /dev/null
		else
			find . -exec stat "$sf" "$sx" {} + 2> /dev/null
		fi | sed 's/^/S /'
		find . -type l -exec sh -c 'for f; do printf "L %s\t%s\n" "$f" "$(readlink "$f")"; done' sh {} + 2> /dev/null
		find . -type f -exec cksum {} + 2> /dev/null | sed 's/^/C /'
	} | awk -v fmt="$1" -v now="$now" '
	function rest(s, n,   i) { for (i = 0; i < n; i++) sub(/^[^ ]* /, "", s); return s }
	function whole(t,   p) { split(t, p, "."); return p[1] }
	function tag(t,   d) { d = now - whole(t); return (d >= -5 && d <= 120) ? "now" : t }
	$1 == "S" && NF >= 8 { n = rest($0, 7); mode[n] = $2; uid[n] = $3; gid[n] = $4; size[n] = $5; mt[n] = $6; at[n] = $7 }
	$1 == "L" { split($0, a, "\t"); n = substr(a[1], 3); target[n] = a[2]; islink[n] = 1 }
	$1 == "C" && NF >= 4 { n = rest($0, 3); sum[n] = $2 }
	END {
		for (n in mode) {
			if (fmt == "chmod") { if (n in islink) print "L " n " -> " target[n]; else print "F " n " " mode[n] }
			else if (fmt == "chown") { if (n in islink) print "L " n " -> " target[n] " " uid[n] " " gid[n]; else print "F " n " " mode[n] " " uid[n] " " gid[n] }
			else if (fmt == "cp") { if (n in islink) print n " -> " target[n] " m=" tag(whole(mt[n])); else print n " " mode[n] " size=" size[n] " m=" tag(whole(mt[n])) " a=" tag(whole(at[n])) " sum=" sum[n] }
			else if (fmt == "touch") { if (n in islink) print n " -> " target[n]; else print n " m=" tag(mt[n]) " a=" tag(at[n]) " size=" size[n] " mode=" mode[n] }
		}
	}' | sort
}
