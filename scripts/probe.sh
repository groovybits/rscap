#!/bin/sh
#
if [ "$RUST_BACKTRACE" = "" ]; then
    BACKTRACE=full
else
    BACKTRACE=$RUST_BACKTRACE
fi
if [ -f ".env" ]; then
    source "./.env"
fi
if [ "$BUILD" == "" ]; then
    BUILD=release-with-debug
fi
if [ "$GST_DEBUG_LEVEL" == "" ]; then
    GST_DEBUG_LEVEL=3
fi
if [ "$SOURCE_IP" == "" ]; then
    SOURCE_IP=224.0.0.200
fi
if [ "$SOURCE_DEVICE" == "" ]; then
    SOURCE_DEVICE=eth0
fi
if [ "$SOURCE_PORT" == "" ]; then
    SOURCE_PORT=10001
fi
if [ "$TARGET_PORT" == "" ]; then
    TARGET_PORT=5556
fi
if [ "$TARGET_HOST" == "" ]; then
    TARGET_HOST=0.0.0.0
fi
GST_PLUGIN_PATH=/opt/rscap/lib64/gstreamer-1.0
LD_LIBRARY_PATH=/opt/rscap/lib64:$LD_LIBRARY_PATH
PATH=/opt/rscap/bin:$PATH
if [ -f "target/$BUILD/probe" ]; then
    PROBE_BIN=target/$BUILD/probe
elif [ -f "/opt/rscap/bin/probe" ]; then
    PROBE_BIN=/opt/rscap/bin/probe
else
    echo "No probe binary found in /opt/rscap/bin or target/$BUILD/"
    exit 1
fi

echo "Using $PROBE_BIN"

$PROBE_BIN -V

#VALGRIND="valgrind --show-leak-kinds=definite,possible --leak-check=full"

sudo GST_PLUGIN_PATH=$GST_PLUGIN_PATH \
    LD_LIBRARY_PATH=$LD_LIBRARY_PATH \
    GST_DEBUG=$GST_DEBUG_LEVEL \
    RUST_BACKTRACE=$BACKTRACE \
    $VALGRIND $PROBE_BIN \
    --pcap-stats \
    --source-ip $SOURCE_IP \
    --source-port $SOURCE_PORT \
    --source-device $SOURCE_DEVICE \
    --target-ip 0.0.0.0 \
    --send-null-packets \
    --use-wireless \
    --target-port $TARGET_PORT \
    --extract-images \
    $@
