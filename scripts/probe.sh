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
GST_PLUGIN_PATH=/opt/rscap/lib64/gstreamer-1.0
PROBE_BIN=/target/$BUILD/probe
if [ -f "/opt/rscap/bin/probe" ]; then
    PROBE_BIN=/opt/rscap/bin/probe
fi

sudo GST_PLUGIN_PATH=$GST_PLUGIN_PATH \
    GST_DEBUG=$GST_DEBUG_LEVEL \
    RUST_BACKTRACE=$BACKTRACE \
    RUST_LOG=$LOGLEVEL \
    $PROBE_BIN \
    --pcap-stats \
    --source-ip $SOURCE_IP \
    --source-port $SOURCE_PORT \
    --source-device $SOURCE_DEVICE \
    --target-ip 0.0.0.0 \
    --send-null-packets \
    --target-port $TARGET_PORT \
    --extract-images \
    --zmq-batch-size 10000 $@
