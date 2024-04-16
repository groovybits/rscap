#!/bin/sh
#
if [ -f "scripts/setup_env.sh" ]; then
    source scripts/setup_env.sh
elif [ -f "/opt/rscap/bin/setup_env.sh" ]; then
    source /opt/rscap/bin/setup_env.sh
fi

BUILD=release-with-debug
OUTPUT_FILE=images/test.jpg
KAFKA_BROKER=sun:9092

MONITOR_BIN=/target/$BUILD/monitor

if [ -f "target/$BUILD/monitor" ]; then
    MONITOR_BIN=target/$BUILD/monitor
elif [ -f "/opt/rscap/bin/monitor" ]; then
    MONITOR_BIN=$PREFIX/bin/monitor
else
    echo "No monitor binary found in /opt/rscap/bin or target/$BUILD/"
    exit 1
fi

echo "Using $MONITOR_BIN"
$MONITOR_BIN -V

GST_PLUGIN_PATH=$GST_PLUGIN_PATH \
    GST_DEBUG=$GST_DEBUG_LEVEL \
    LD_LIBRARY_PATH=$LD_LIBRARY_PATH \
    RUST_BACKTRACE=$BACKTRACE \
    $MONITOR_BIN \
    --kafka-broker $KAFKA_BROKER \
    --send-to-kafka \
    --output-file $OUTPUT_FILE $@
