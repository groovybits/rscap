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
IMAGE_HEIGHT=96
IMAGE_RATE=333333000
FILMSTRIP_FRAMES=18
LD_LIBRARY_PATH=/opt/rscap/lib64:$LD_LIBRARY_PATH
if [ -f "target/$BUILD/probe" ]; then
    PROBE_BIN=target/$BUILD/probe
elif [ -f "/opt/rscap/bin/probe" ]; then
    PROBE_BIN=/opt/rscap/bin/probe
else
    echo "No probe binary found in /opt/rscap/bin or target/$BUILD/"
    exit 1
fi

$PROBE_BIN -V

sudo GST_PLUGIN_PATH=$GST_PLUGIN_PATH \
    LD_LIBRARY_PATH=$LD_LIBRARY_PATH \
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
    --filmstrip-length $FILMSTRIP_FRAMES \
    --image-sample-rate-ns $IMAGE_RATE \
    --image-height $IMAGE_HEIGHT \
    --zmq-batch-size 10000 $@
