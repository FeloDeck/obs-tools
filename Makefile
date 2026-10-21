PLUGIN   = de.felitendo.obs-tools.sdPlugin
VDECK    = de.felitendo.virtual-deck.sdPlugin
VERSION := $(shell cat VERSION)
TARGET  := $(or $(CARGO_TARGET_DIR),target)

# Native OpenDeck. On macOS this is in ~/Library/Application Support.
# For the Linux Flatpak use:
#   make install PLUGINS=~/.var/app/me.amankhanna.opendeck/config/opendeck/plugins
UNAME_S := $(shell uname -s)
ifeq ($(UNAME_S),Darwin)
PLUGINS ?= $(HOME)/Library/Application Support/opendeck/plugins
else
PLUGINS ?= $(HOME)/.config/opendeck/plugins
endif

.PHONY: build install uninstall link install-virtual-deck uninstall-virtual-deck package check icons manifest clean

# The plugin for this PC, into $(PLUGIN)/bin/<target>/.
build:
	cargo xtask build

install: build
	rm -rf "$(PLUGINS)/$(PLUGIN)"
	mkdir -p "$(PLUGINS)"
	cp -r $(PLUGIN) "$(PLUGINS)/$(PLUGIN)"
	@echo "Installed. Restart OpenDeck (or reload the plugin in its settings)."

uninstall:
	rm -rf "$(PLUGINS)/$(PLUGIN)"

# For development: OpenDeck runs the files from this folder. Needs developer mode in OpenDeck.
link: build
	rm -rf "$(PLUGINS)/$(PLUGIN)"
	ln -s "$(CURDIR)/$(PLUGIN)" "$(PLUGINS)/$(PLUGIN)"

install-virtual-deck:
	cargo build --release --package virtual-deck
	rm -rf "$(PLUGINS)/$(VDECK)"
	cp -r tools/virtual-deck/$(VDECK) "$(PLUGINS)/$(VDECK)"
	cp "$(TARGET)/release/virtual-deck" "$(PLUGINS)/$(VDECK)/virtual-deck"
	@echo "Installed. Restart OpenDeck, then open http://127.0.0.1:57190/"

uninstall-virtual-deck:
	rm -rf "$(PLUGINS)/$(VDECK)"

# dist/de.felitendo.obs-tools.streamDeckPlugin for x86_64 and ARM64, installable from the
# OpenDeck plugin page. Needs zig and cargo-zigbuild. Only on a Mac it has the macOS binaries too.
package:
	cargo xtask package

check:
	cargo test --workspace

icons:
	cargo xtask fit-glyphs
	cargo xtask icons

manifest:
	cargo xtask manifest

clean:
	rm -rf dist $(PLUGIN)/bin
	cargo clean
