#!/bin/bash
#
target/release/monitor --no-progress --recv-raw-stream --send-to-kafka --output-file capture.ts
