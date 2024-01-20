#!/bin/bash
#
ERROR_LEVEL=error
CPU_BIND=0
MEM_BIND=0

## SMPTE2110
sudo RUST_LOG=$ERROR_LEVEL numactl --cpubind=$CPU_BIND --membind=$MEM_BIND \
    target/release/probe \
        --pcap-stats \
        --smpte2110
