PREFIX ?= $(HOME)/.local

.PHONY: all check fmt lint lua fmt-lua test run install install-bin install-runtime

all: check

check: fmt lua lint test

# Every Lua file a layer can load: the shipped runtime, the examples, and the plugins the tests
# drive. `.stylua.toml` and `.luacheckrc` at the root say how.
LUA := runtime examples neotmux/tests/fixtures

fmt:
	cargo fmt --all --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

lua:
	stylua --check $(LUA)
	luacheck -q $(LUA)

fmt-lua:
	stylua $(LUA)

test:
	cargo test --workspace

run:
	cargo run -p neotmux

# `cargo install` copies the binary and nothing else, so the shipped runtime layer needs its
# own step. It lands next to the binary as ../share/neotmux, which is already on the path
# `Layers::resolve` searches.
install: install-bin install-runtime

install-bin:
	cargo install --path neotmux --root $(PREFIX)

# `runtime/lua` goes over wholesale rather than file by file: enumerating it once shipped a
# layer whose `init.lua` required a module the install had never copied, and every start of
# an installed neotmux raised on it.
# `PREFIX=` on the command line would make the `rm -rf` below read `/share/neotmux/lua`.
install-runtime:
	$(if $(strip $(PREFIX)),,$(error PREFIX is empty))
	install -d $(PREFIX)/share/neotmux
	install -m 644 runtime/init.lua $(PREFIX)/share/neotmux/init.lua
	rm -rf $(PREFIX)/share/neotmux/lua
	cp -R runtime/lua $(PREFIX)/share/neotmux/lua
	chmod -R a+rX $(PREFIX)/share/neotmux/lua
# The sounds go with the layer they are resolved against: `plugin.herdr.chime` finds them beside
# the file `require` loaded it from, so an install that copied only the Lua would ship a
# plugin that cannot play anything.
	rm -rf $(PREFIX)/share/neotmux/sounds
	cp -R runtime/sounds $(PREFIX)/share/neotmux/sounds
	chmod -R a+rX $(PREFIX)/share/neotmux/sounds
