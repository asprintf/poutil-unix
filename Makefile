# "[" is the program test again: it looks at the name it was run as
all:
	cargo build --release
	ln -f target/release/test 'target/release/['

# unit tests, then the checks that need real processes (see difftest/regress.sh);
# the comparison with other implementations is separate: sh difftest/difftest.sh
test: all
	cargo test
	sh difftest/regress.sh

clean:
	cargo clean

# Install the built utilities where the PCTS runs (pcts-run --ours) find
# them. Root-owned so that the vsc user cannot alter them. Asks for sudo.
VSC_USER ?= vsc
VSC_GROUP ?= vscgrp1
OURS ?= /Users/$(VSC_USER)/ours/bin
BINS = true false echo basename dirname pwd sleep logname tty uname unlink link cat \
	tee rmdir mkdir mkfifo pathchk env nohup nice time kill \
	wc head tail tsort uniq expr sort test '[' expand unexpand

deploy: all
	sudo install -d -o $(VSC_USER) -g $(VSC_GROUP) $(OURS)
	cd target/release && sudo install -o root -g wheel -m 755 $(BINS) $(OURS)/

.PHONY: all test clean deploy
