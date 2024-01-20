#!/bin/bash
#
rustfmt --edition 2021 src/bin/probe.rs
rustfmt --edition 2021 src/bin/monitor.rs
rustfmt --edition 2021 build.rs
rustfmt --edition 2021 src/stream_data.rs
rustfmt --edition 2021 src/lib.rs
