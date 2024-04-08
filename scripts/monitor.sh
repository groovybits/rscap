#!/bin/bash
#
BUILD=release-with-debug
OUTPUT_FILE=images/test.jpg
KAFKA_BROKER=sun:9092

RUST_LOG=info target/$BUILD/monitor \
    --kafka-broker $KAFKA_BROKER \
    --output-file $OUTPUT_FILE $@
