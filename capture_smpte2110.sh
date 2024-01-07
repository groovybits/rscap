#!/bin/bash

## SMPTE2110
target/release/probe --no-progress --pcap-stats --batch-size 1 --buffer-size 10000000000 --pcap-channel-size 100000 --zmq-channel-size 10000 --packet-size 1250 --immediate-mode --no-zmq
