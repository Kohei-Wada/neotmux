PREFIX ?= $(HOME)/.local

.PHONY: all check fmt lint test run install

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
install:
	cargo install --path crates/neotmux --root $(PREFIX)
	install -d $(PREFIX)/share/neotmux/lua/plugin
	install -m 644 runtime/init.lua $(PREFIX)/share/neotmux/init.lua
	install -m 644 runtime/lua/plugin/*.lua $(PREFIX)/share/neotmux/lua/plugin/
