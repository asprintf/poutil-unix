# libiconv says nothing under -c; ours writes one message at the end unless -s
-c -f UTF-8 -t ASCII accent_mid
-c -f UTF-8 -t ASCII utf8
-c -f UTF-8 -t ISO8859-1 emoji
-c -f UTF-8 -t UTF-16BE bad_cont
-c -f UTF-8 -t UTF-16BE bad_in_emoji
-c -f UTF-8 -t UTF-16BE bad_mid
-c -f UTF-8 -t UTF-16BE euros_bad
-c -f UTF-8 -t UTF-8 bad_mid
-c -f UTF-8 -t UTF-8 bad_mid ascii bad_mid
# libiconv goes on with the next file after an invalid input; ours stops
-f UTF-8 -t UTF-16BE bad_mid ascii
# libiconv's iconv does not take an option argument written next to its option
-fISO8859-1 -tUTF-8 latin1
# with no -f and -t in the C locale Apple's libiconv here replaces bytes it cannot read, libiconv in MacPorts refuses them
utf8
