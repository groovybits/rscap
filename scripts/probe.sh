#!/bin/bash
#
sudo RUST_LOG=info \
    target/release/probe \
    --pcap-stats \
    --source-ip 224.0.0.200 \
    --source-port 10000 \
    --source-device eth0  \
    --target-ip 0.0.0.0 \
    --send-null-packets \
    --target-port 5556 \
    --extract-images \
    --zmq-batch-size 10000 $@
