#!/bin/bash
#
ERROR_LEVEL=error
CPU_BIND=0
MEM_BIND=0

## MPEGTS
RUST_LOG=$ERROR_LEVEL numactl --cpubind=$CPU_BIND --membind=$MEM_BIND \
    target/release/probe \
        --no-progress \
        --pcap-stats \
        --immediate-mode \
        --show-tr101290 \
        --send-raw-stream
