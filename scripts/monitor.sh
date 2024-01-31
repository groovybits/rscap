#!/bin/bash
#
target/release/monitor \
    --send-to-kafka # --recv-raw-stream --output-file capture.ts
