.PHONY: all clean probe_run build install

all: build

clean:
	cargo clean

probe_run:
	sh scripts/probe.sh

build:
	sh scripts/compile.sh

install:
	sh scripts/install.sh
