#!/bin/bash
#
# Best Buffer Source Find BBSF
#

# Initial buffer size
buffer_size=10000000
# Loop until buffer size is greater than 0
while [ $buffer_size -gt 0 ]
do
    echo "$buffer_size bytes pcap buffer..."
    # Run the probe command with the current buffer size
    output=$(sudo RUST_LOG=info target/release/probe --no-progress --packet-count 1000000 --packet-size 1500 --buffer-size $buffer_size)

    # Extract packet loss information from the output
    received=$(echo "$output" | grep -oP 'Received: \K\d+')
    dropped=$(echo "$output" | grep -oP 'Dropped: \K\d+')
    iface_dropped=$(echo "$output" | grep -oP 'Interface Dropped: \K\d+')

    # Print the current buffer size and packet loss details
    echo "Buffer Size: $buffer_size, Received: $received, Dropped: $dropped, Interface Dropped: $iface_dropped"

    # Decrement the buffer size
    buffer_size=$((buffer_size - 125000))
done
