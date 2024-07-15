#!/bin/sh
#

run_command() {
    if command -v sudo &>/dev/null; then
        sudo "$@"
    else
        "$@"
    fi
}

if [ -f "scripts/setup_env.sh" ]; then
    source scripts/setup_env.sh
elif [ -f "/opt/rsprobe/bin/setup_env.sh" ]; then
    source /opt/rsprobe/bin/setup_env.sh
fi

if [ -f ".env" ]; then
    source "./.env"
fi

if [ "$OUTPUT_FILE" == "" ]; then
    OUTPUT_FILE=images/test.jpg
fi

if [ "$KAFKA_BROKER" == "" ]; then
    KAFKA_BROKER=localhost:9092
fi

if [ "$RUST_BACKTRACE" == "" ]; then
    BACKTRACE=full
fi

if [ "$BUILD" == "" ]; then
    BUILD=release-with-debug
fi
if [ "$GST_DEBUG_LEVEL" == "" ]; then
    GST_DEBUG_LEVEL=1
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
if [ -f "target/$BUILD/probe" ]; then
    PROBE_BIN=target/$BUILD/probe
elif [ -f "$PREFIX/bin/probe" ]; then
    PROBE_BIN=$PREFIX/bin/probe
else
    echo "No probe binary found in $PREFIX/bin or target/$BUILD/"
    exit 1
fi

EXTRACT_IMAGES_ARG=
#EXTRACT_IMAGES_ARG=--extract-images

echo "Using $PROBE_BIN"

$PROBE_BIN -V

#VALGRIND="valgrind --leak-check=full --show-leak-kinds=all --track-origins=yes --log-file=valgrind.log "

CPU_BIND=0
MEM_BIND=0
# NUMACTL="numactl --cpubind=$CPU_BIND --membind=$MEM_BIND"
#    --kafka-broker $KAFKA_BROKER \

#run_command \ #FIXME 
GST_PLUGIN_PATH=$GST_PLUGIN_PATH \
    LD_LIBRARY_PATH=$LD_LIBRARY_PATH \
    GST_DEBUG=$GST_DEBUG_LEVEL \
    RUST_BACKTRACE=$BACKTRACE \
    RUST_LOG=$RUST_LOG \
    $VALGRIND $NUMACTL $PROBE_BIN \
    $EXTRACT_IMAGES_ARG \
    --source-device $SOURCE_DEVICE \
    --pcap-stats \
    --source-ip $SOURCE_IP \
    --source-port $SOURCE_PORT \
    --kafka-topic "rsprobe" \
    --output-file $OUTPUT_FILE \
    $@
