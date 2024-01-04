#!/bin/bash
#
# Best Buffer Source Find BBSF
#

# check for -h on the command line and output help if given and exit
if [ "$1" == "-h" ]; then
    echo "Best Buffer Source Find BBSF"
    echo "  This script will run the probe command with a decreasing buffer size until the probe command fails."
    echo "  The last successful buffer size will be the best buffer size to use for the current system."
    echo ""
    echo "  The probe command will be run using the following environment variables..."
    echo ""
    echo "Usage: [buffer_size=N] [increment_size=N] [packet_size=N] [packet_count=N] $0"
    echo "  buffer_size:    The initial buffer size to start with. Default: 10000000"
    echo "  increment_size: The amount to decrement the buffer size by. Default: 125000"
    echo "  packet_size:    The size of the packets to send. Default: 1500"
    echo "  packet_count:   The number of packets to send. Default: 1000000"
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
    echo "---"
    echo "$buffer_size bytes pcap buffer..."
    # Run the probe command with the current buffer size
    output=$(sudo RUST_LOG=info target/release/probe --no-progress --packet-count $packet_count --packet-size $packet_size --buffer-size $buffer_size)
    echo "$output"

    # Decrement the buffer size
    buffer_size=$((buffer_size - increment_size))
done
