.PHONY: all clean probe_run build build_gst install

all: build

clean:
	cargo clean

probe_run:
	sh scripts/probe.sh

build:
	sh scripts/compile.sh

build_gst:
	sh scripts/compile.sh gst

install:
	sh scripts/install.sh

