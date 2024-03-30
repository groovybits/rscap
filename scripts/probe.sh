#!/bin/bash
#
RUST_LOG=info \
    target/release/probe \
    --pcap-stats \
    --source-ip 224.0.0.200 \
    --source-port 10000 \
    --source-device en0  \
    --target-ip 0.0.0.0 \
    --target-port 5556 \
    --zmq-batch-size 10000 $@
