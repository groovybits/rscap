#!/bin/bash
#
target/release/monitor \
    --no-progress \
    --recv-json-header \
    --recv-raw-stream \
    --output-file capture.ts \
    --send-to-kafka
