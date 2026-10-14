PLUGIN   = de.felitendo.obs-tools.sdPlugin
VDECK    = de.felitendo.virtual-deck.sdPlugin
VERSION := $(shell cat VERSION)

# Native OpenDeck. For the Flatpak use:
#   make install PLUGINS=~/.var/app/me.amankhanna.opendeck/config/opendeck/plugins
PLUGINS ?= $(HOME)/.config/opendeck/plugins

.PHONY: install uninstall link install-virtual-deck uninstall-virtual-deck package check icons manifest clean

install:
	rm -rf "$(PLUGINS)/$(PLUGIN)"
	mkdir -p "$(PLUGINS)"
	cp -r $(PLUGIN) "$(PLUGINS)/$(PLUGIN)"
	@echo "Installed. Restart OpenDeck (or reload the plugin in its settings)."

uninstall:
	rm -rf "$(PLUGINS)/$(PLUGIN)"

# For development: OpenDeck runs the files from this folder. Needs developer mode in OpenDeck.
link:
	rm -rf "$(PLUGINS)/$(PLUGIN)"
	ln -s "$(CURDIR)/$(PLUGIN)" "$(PLUGINS)/$(PLUGIN)"

install-virtual-deck:
	rm -rf "$(PLUGINS)/$(VDECK)"
	cp -r tools/virtual-deck/$(VDECK) "$(PLUGINS)/$(VDECK)"
	@echo "Installed. Restart OpenDeck, then open http://127.0.0.1:57190/"

uninstall-virtual-deck:
	rm -rf "$(PLUGINS)/$(VDECK)"

# dist/de.felitendo.obs-tools.streamDeckPlugin, installable from the OpenDeck plugin page.
package:
	rm -rf dist && mkdir -p dist
	python3 -c 'import shutil; shutil.make_archive("dist/obstools", "zip", ".", "$(PLUGIN)")'
	mv dist/obstools.zip dist/de.felitendo.obs-tools.streamDeckPlugin
	@echo "dist/de.felitendo.obs-tools.streamDeckPlugin ($(VERSION))"

check:
	node --test 'test/*.test.mjs'

icons:
	node scripts/fit-glyphs.mjs
	node scripts/build-icons.mjs

manifest:
	node scripts/manifest.mjs

clean:
	rm -rf dist
