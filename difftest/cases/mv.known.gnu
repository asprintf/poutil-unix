# A negative answer (or end of input) to the question of -i: "do nothing more with the current source_file and go
# on". Ours exits 0 as macOS does (nothing failed; same as our cp -i); GNU 9 exits 1.
in:no -i f g
in:eof -i f g
in:no -fi f g
in:no -f -i f g
in:no -i -i f g
