#!/bin/bash
#
BUILD=release-with-debug
OUTPUT_FILE=images/test.jpg
KAFKA_BROKER=sun:9092

GST_PLUGIN_PATH=/opt/rscap/lib64/gstreamer-1.0
MONITOR_BIN=/target/$BUILD/monitor
if [ -f "/opt/rscap/bin/probe" ]; then
    MONITOR_BIN=/opt/rscap/bin/monitor
fi

GST_PLUGIN_PATH=$GST_PLUGIN_PATH \
    GST_DEBUG=$GST_DEBUG_LEVEL \
    RUST_BACKTRACE=$BACKTRACE \
    RUST_LOG=$LOGLEVEL \
    $MONITOR_BIN \
    --kafka-broker $KAFKA_BROKER \
    --send-to-kafka \
    --output-file $OUTPUT_FILE $@
