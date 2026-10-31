all:
	cargo build --release

test:
	cargo test

clean:
	cargo clean

# Install the built utilities where the PCTS runs (pcts-run --ours) find
# them. Root-owned so that the vsc user cannot alter them. Asks for sudo.
VSC_USER ?= vsc
VSC_GROUP ?= vscgrp1
OURS ?= /Users/$(VSC_USER)/ours/bin
BINS = true false echo basename dirname pwd sleep logname tty uname unlink link cat \
	tee rmdir mkdir mkfifo pathchk env nohup nice time kill

deploy: all
	sudo install -d -o $(VSC_USER) -g $(VSC_GROUP) $(OURS)
	cd target/release && sudo install -o root -g wheel -m 755 $(BINS) $(OURS)/

.PHONY: all test clean deploy
