PREFIX ?= $(HOME)/.local

.PHONY: all check fmt lint test run install install-bin install-runtime

all: check

check: fmt lint test

fmt:
	cargo fmt --all --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

run:
	cargo run -p neotmux

# `cargo install` copies the binary and nothing else, so the shipped runtime layer needs its
# own step. It lands next to the binary as ../share/neotmux, which is already on the path
# `Layers::resolve` searches.
install: install-bin install-runtime

install-bin:
	cargo install --path crates/neotmux --root $(PREFIX)

# `runtime/lua` goes over wholesale rather than file by file: enumerating it once shipped a
# layer whose `init.lua` required a module the install had never copied, and every start of
# an installed neotmux raised on it.
install-runtime:
	install -d $(PREFIX)/share/neotmux
	install -m 644 runtime/init.lua $(PREFIX)/share/neotmux/init.lua
	rm -rf $(PREFIX)/share/neotmux/lua
	cp -R runtime/lua $(PREFIX)/share/neotmux/lua
	chmod -R a+rX $(PREFIX)/share/neotmux/lua
