#!/bin/bash
#
# Best Buffer Source Find BBSF
#

# check for -h on the command line and output help if given and exit
if [ "$1" == "-h" ]; then
    echo "Usage: $0 [buffer_size] [increment_size] [packet_size] [packet_count]"
    echo "  buffer_size:    The initial buffer size to start with"
    echo "  increment_size: The amount to decrement the buffer size by"
    echo "  packet_size:    The size of the packets to send"
    echo "  packet_count:   The number of packets to send"
    exit 0
fi

# check if environment overrides buffer size
buffer_size=${buffer_size:-10000000}

# check if increment size is defined
increment_size=${increment_size:-125000}

# check if environment overrides packet-size
packet_size=${packet_size:-1500}

# check if environment overrides packet-count
packet_count=${packet_count:-1000000}

# Loop until buffer size is greater than 0
while [ $buffer_size -gt 0 ]
do
    echo "$buffer_size bytes pcap buffer..."
    # Run the probe command with the current buffer size
    output=$(sudo RUST_LOG=info target/release/probe --no-progress --packet-count $packet_count --packet-size $packet_size --buffer-size $buffer_size)

    # Extract packet loss information from the output
    received=$(echo "$output" | grep -oP 'Received: \K\d+')
    dropped=$(echo "$output" | grep -oP 'Dropped: \K\d+')
    iface_dropped=$(echo "$output" | grep -oP 'Interface Dropped: \K\d+')

    # Print the current buffer size and packet loss details
    echo "Buffer Size: $buffer_size, Received: $received, Dropped: $dropped, Interface Dropped: $iface_dropped"

    # Decrement the buffer size
    buffer_size=$((buffer_size - increment_size))
done
