#!/bin/sh
#
if [ -f "scripts/setup_env.sh" ]; then
    source scripts/setup_env.sh
elif [ -f "/opt/rscap/bin/setup_env.sh" ]; then
    source /opt/rscap/bin/setup_env.sh
fi

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
    SOURCE_PORT=10000
fi
if [ "$TARGET_PORT" == "" ]; then
    TARGET_PORT=5556
fi
if [ "$TARGET_HOST" == "" ]; then
    TARGET_HOST=0.0.0.0
fi
if [ -f "target/$BUILD/probe" ]; then
    PROBE_BIN=target/$BUILD/probe
elif [ -f "$PREFIX/bin/probe" ]; then
    PROBE_BIN=$PREFIX/bin/probe
else
    echo "No probe binary found in $PREFIX/bin or target/$BUILD/"
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
    --target-port $TARGET_PORT \
    --extract-images \
    $@
