.PHONY: all clean probe_run monitor_run build gst_install

all: build

clean:
	cargo clean

probe_run:
	scripts/probe.sh

monitor_run:
	scripts/monitor.sh


build:
	scripts/compile.sh

build_gst:
	scripts/compile.sh gst

gst_install:
	scripts/install_gstreamer.sh

