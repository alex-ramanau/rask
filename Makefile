# Local builds of rask, and the snap (see docs/snap.md).
#
#   make            release build: $(TARGET_DIR)/release/rask
#   make check      what CI's cargo job runs: fmt, clippy, tests
#   make snap       the snap, built by snapcraft in LXD
#   make help       all targets

CARGO ?= cargo
SNAPCRAFT ?= snapcraft

# Honour CARGO_TARGET_DIR, as the scripts do (needed on a VirtualBox shared
# folder, see README.md).
TARGET_DIR := $(or $(CARGO_TARGET_DIR),target)

PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin

VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
ARCH := $(shell dpkg --print-architecture 2>/dev/null || uname -m)
SNAP_FILE := rask_$(VERSION)_$(ARCH).snap

# The same static target the snap builds.
MUSL_TARGET := $(shell uname -m)-unknown-linux-musl

# snap-destructive builds in a copy of the repo here: on a shared folder the
# linker writes corrupt binaries (docs/snap.md).
SNAP_BUILD_DIR ?= $(HOME)/.cache/rask-snap-build

.DEFAULT_GOAL := release

.PHONY: help build release musl test fmt fmt-check clippy check install uninstall \
	snap snap-destructive snap-install clean clean-snap

help:
	@echo 'Local builds:'
	@echo '  build             debug build'
	@echo '  release           release build (default)'
	@echo '  musl              static release build for $(MUSL_TARGET), as in the snap'
	@echo '  install           install the release build to $(BINDIR)'
	@echo '  uninstall         remove $(BINDIR)/rask'
	@echo 'Checks:'
	@echo '  test              cargo test --all (set ACK3_DIR for the ack3 fixture tests)'
	@echo '  fmt / fmt-check   format the code / check formatting'
	@echo '  clippy            clippy with warnings as errors'
	@echo '  check             fmt-check, clippy and test, as CI does'
	@echo 'Snap ($(SNAP_FILE)):'
	@echo '  snap              snapcraft pack in LXD'
	@echo '  snap-destructive  snapcraft pack on this host (Ubuntu 24.04), in $(SNAP_BUILD_DIR)'
	@echo '  snap-install      install $(SNAP_FILE) (sudo)'
	@echo 'Cleanup:'
	@echo '  clean             cargo clean'
	@echo '  clean-snap        snapcraft clean, remove snap build files and *.snap'

build:
	$(CARGO) build

release:
	$(CARGO) build --release --locked -p rask
	@echo 'Built $(TARGET_DIR)/release/rask'

musl:
	rustup target add $(MUSL_TARGET)
	$(CARGO) build --release --locked -p rask --target $(MUSL_TARGET)
	@echo 'Built $(TARGET_DIR)/$(MUSL_TARGET)/release/rask'

test:
	$(CARGO) test --all

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all --check

clippy:
	$(CARGO) clippy --all-targets -- -D warnings

check: fmt-check clippy test

install: release
	install -Dm755 $(TARGET_DIR)/release/rask $(DESTDIR)$(BINDIR)/rask

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/rask

snap:
	$(SNAPCRAFT) pack --output $(SNAP_FILE)

snap-destructive:
	mkdir -p $(SNAP_BUILD_DIR)
	rsync -a --delete --exclude=/.git/ --exclude=/target/ --exclude=/tmp/ \
		--exclude=/parts/ --exclude=/stage/ --exclude=/prime/ --exclude='*.snap' \
		./ $(SNAP_BUILD_DIR)/
	cd $(SNAP_BUILD_DIR) && $(SNAPCRAFT) pack --destructive-mode --output $(SNAP_FILE)
	cp $(SNAP_BUILD_DIR)/$(SNAP_FILE) .
	@echo 'Built $(SNAP_FILE)'

snap-install:
	sudo snap install --dangerous --classic $(SNAP_FILE)

clean:
	$(CARGO) clean

clean-snap:
	-$(SNAPCRAFT) clean
	rm -rf parts stage prime $(SNAP_BUILD_DIR)
	rm -f *.snap
