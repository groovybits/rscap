.PHONY: all clean probe_run build build_gst docker docker_up docker_down install

all: build

clean:
	cargo clean

probe_run:
	sh scripts/probe.sh

build:
	sh scripts/compile.sh

build_gst:
	sh scripts/compile.sh gst

docker:
	docker compose build

docker_up:
	docker compose up

docker_down:
	docker compose down

install:
	sh scripts/install.sh
