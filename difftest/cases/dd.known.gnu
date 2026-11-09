# A size is a number, followed by k or b: k alone is none (GNU takes it as 1024).
if=big ibs=k
# block and unblock need cbs (the standard leaves it open without one): an error here, GNU copies the data.
if=lines conv=block
if=lines conv=unblock
# swab swaps the pairs of each input block and leaves the last byte of an odd block alone (the standard); GNU carries it over to the next block.
if=odd bs=3 conv=swab
if=odd ibs=1 conv=swab
if=odd ibs=3 conv=swab
