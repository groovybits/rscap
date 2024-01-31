#!/bin/bash
#
# Load DPDK Modules
set -e

echo "Loading DPDK modules..."

DEVICE={device:-eth0}
sudo modprobe uio
sudo insmod kmod/igb_uio.ko

# Bind Network Devices to DPDK
echo "Binding network devices to DPDK..."
sudo usertools/dpdk-devbind.py --bind=igb_uio $DEVICE

# Done
echo "DPDK modules are loaded."
