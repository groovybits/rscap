#!/bin/bash
#
MODE=release
LOGLEVEL=info
GST_DEBUG_LEVEL=0
BACKTRACE=full
sudo GST_DEBUG=$GST_DEBUG_LEVEL RUST_BACKTRACE=$BACKTRACE RUST_LOG=$LOGLEVEL \
    target/$MODE/probe \
    --pcap-stats \
    --source-ip 224.0.0.200 \
    --source-port 10000 \
    --source-device eth0  \
    --target-ip 0.0.0.0 \
    --send-null-packets \
    --target-port 5556 \
    --zmq-batch-size 10000 $@
