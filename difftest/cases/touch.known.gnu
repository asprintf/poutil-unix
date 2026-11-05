# Times the file system cannot hold. ext4 stores 1901-12-13 to 2446 and clamps what is outside; GNU
# touch passes the clamped time on and says yes. The spec says touch shall exit with an error when the
# time cannot be represented as the file's timestamp, so ours reads the times back and refuses (the same
# thing as the APFS lines in touch.known). Where the limits are depends on the file system.
-t 190001010000 f
-d 1900-01-01T00:00:00Z f
-t 190001010000.00 f
-t 190002280000 f
-t 190101010000 f
-t 186001010000 f
-t 170001010000 f
-t 000101010000 f
-t 000101010000.00 f
-t 010101010000 f
-d 1900-01-01T00:00:00.5Z f
-d 1900-12-31T23:59:59Z f
-d 0001-01-01T00:00:00Z f
-d 0100-03-01T00:00:00Z f
-d 1000-01-01T00:00:00Z f
-d 1582-10-15T00:00:00Z f
-d 1582-10-04T00:00:00Z f
-d 1677-09-21T00:12:43Z f
-d 12345-01-01T00:00:00Z f
-d 0000-01-01T00:00:00Z f
# -d date_time is the ISO 8601 form of the spec and nothing else. GNU parses free-form dates too
# (a year alone, "today", "Nov 12 2007", @SECONDS, a time zone offset, blanks around, a missing
# digit); ours refuses them, as the spec lists only the one form.
-d 2007 f
-d 2007-11-12T10:15:3 f
-d 2007-11-1T10:15:30 f
-d '2007-11-12  10:15:30' f
-d '2007-11-12T10:15:30 Z' f
-d 2007-11-12T10:15:30-01:00 f
-d '2007-11-12T10:15:30 ' f
-d '  2007-11-12T10:15:30' f
-d '2007-11-12T10:15:30	' f
-d '2007-11-12	10:15:30' f
-d @1194862530 f
-d today f
-d '2007-11-12T10:15:30 UTC' f
-d 'Nov 12 2007' f
-d '12 Nov 2007 10:15:30' f
-d 2007-11-12T10:15:30Z+0 f
-d 2007-11-12T10:15:30-0 f
# SS may be 60 ("as with -t time", the leap second): ours accepts it, GNU refuses it in -d.
-d 2007-11-12T10:15:60 f
-d 1998-12-31T23:59:60Z f
-d 2016-12-31T23:59:60Z f
# Beyond what APFS holds (after 2262): GNU says success and the time is clamped; the spec says to exit with an error, as ours does.
-d 2400-02-29T00:00:00Z f
-t 229912312359 f
