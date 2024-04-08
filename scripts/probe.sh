#!/bin/bash
#
BUILD=release-with-debug
LOGLEVEL=info
GST_DEBUG_LEVEL=1
BACKTRACE=full
SOURCE_IP=224.0.0.200
SOURCE_DEVICE=eth0
SOURCE_PORT=10000
TARGET_PORT=5556
sudo GST_DEBUG=$GST_DEBUG_LEVEL RUST_BACKTRACE=$BACKTRACE RUST_LOG=$LOGLEVEL \
    target/$BUILD/probe \
    --pcap-stats \
    --source-ip $SOURCE_IP \
    --source-port $SOURCE_PORT \
    --source-device $SOURCE_DEVICE \
    --target-ip 0.0.0.0 \
    --send-null-packets \
    --target-port $TARGET_PORT \
    --extract-images \
    --zmq-batch-size 10000 $@
