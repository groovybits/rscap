#!/bin/bash
#
ERROR_LEVEL=error
CPU_BIND=0
MEM_BIND=0

## MPEGTS
RUST_LOG=$ERROR_LEVEL numactl --cpubind=$CPU_BIND --membind=$MEM_BIND \
    target/release/probe \
        --pcap-stats \
        --decode-video \
        --debug-nal-types "sps,pps,pic_timing,sei,slice,unknown" \
        --parse-short-nals # --debug-nals --debug-on --show-tr101290
