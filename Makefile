# "[" is the program test again: it looks at the name it was run as
all:
	cargo build --release
	ln -f target/release/test 'target/release/['

# unit tests, then the checks that need real processes (see difftest/regress.sh);
# the comparison with other implementations is separate: sh difftest/difftest.sh
test: all
	cargo test
	sh difftest/regress.sh

# the comparison with other implementations (GNU, BSD) on the case files in difftest/cases;
# one utility: make difftest U=wc
difftest: all
	utils='$(U)'; \
	if [ -z "$$utils" ]; then \
		for f in difftest/cases/*; do case $$f in *.*) ;; *) utils="$$utils $${f##*/}" ;; esac; done; \
	fi; \
	sh difftest/difftest.sh $$utils

# which lines the difftest and regress checks run (needs llvm-tools); output in target/cov/html
coverage:
	sh difftest/coverage.sh $(U)

# the linter, default groups only (see [lints.clippy] in Cargo.toml)
clippy:
	cargo clippy --all-targets

clean:
	cargo clean

# Install the built utilities where the PCTS runs (pcts-run --ours) find
# them. Root-owned so that the vsc user cannot alter them. Asks for sudo.
VSC_USER ?= vsc
VSC_GROUP ?= vscgrp1
OURS ?= /Users/$(VSC_USER)/ours/bin
BINS = true false echo basename dirname pwd sleep logname tty uname unlink link cat \
	tee rm touch date cp ls rmdir mkdir mkfifo pathchk env nohup nice time kill \
	wc head tail tsort uniq expr sort test '[' expand unexpand uuencode uudecode comm cmp cksum id paste fold cut join cal df du printf ln chmod chown xargs

deploy: all
	sudo install -d -o $(VSC_USER) -g $(VSC_GROUP) $(OURS)
	cd target/release && sudo install -o root -g wheel -m 755 $(BINS) $(OURS)/

.PHONY: all test difftest coverage clippy clean deploy
