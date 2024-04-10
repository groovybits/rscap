#!/bin/bash
#
BUILD=release-with-debug
OUTPUT_FILE=images/test.jpg
KAFKA_BROKER=sun:9092

GST_PLUGIN_PATH=/opt/rscap/lib64/gstreamer-1.0
MONITOR_BIN=/target/$BUILD/monitor

LD_LIBRARY_PATH=/opt/rscap/lib64:$LD_LIBRARY_PATH
if [ -f "target/$BUILD/monitor" ]; then
    MONITOR_BIN=target/$BUILD/monitor
elif [ -f "/opt/rscap/bin/monitor" ]; then
    MONITOR_BIN=/opt/rscap/bin/monitor
else
    echo "No monitor binary found in /opt/rscap/bin or target/$BUILD/"
    exit 1
fi

$MONITOR_BIN -V

GST_PLUGIN_PATH=$GST_PLUGIN_PATH \
    GST_DEBUG=$GST_DEBUG_LEVEL \
    LD_LIBRARY_PATH=$LD_LIBRARY_PATH \
    RUST_BACKTRACE=$BACKTRACE \
    RUST_LOG=$LOGLEVEL \
    $MONITOR_BIN \
    --kafka-broker $KAFKA_BROKER \
    --send-to-kafka \
    --output-file $OUTPUT_FILE $@
