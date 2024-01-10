#!/bin/bash
#
target/release/monitor \
    --no-progress \
    --recv-raw-stream \
    --output-file capture.ts \
    --send-to-kafka
