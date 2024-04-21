.PHONY: all clean probe_run build build_gst install

all: build

clean:
	cargo clean

probe_run:
	scripts/probe.sh

build:
	scripts/compile.sh

build_gst:
	scripts/compile.sh gst

install:
	scripts/install.sh

