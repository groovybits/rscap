#!/bin/bash
set -e

# Update System
echo "Updating system..."
sudo yum update -y

# Install Required Dependencies
echo "Installing required dependencies..."
sudo yum groupinstall -y "Development Tools"
sudo yum install -y gcc make kernel-devel numactl-devel python3 python3-pip

# Upgrade pip and Install Meson and Ninja
echo "Upgrading pip and installing Meson and Ninja..."
sudo pip3 install --upgrade pip
sudo pip3 install meson ninja

# Define DPDK Version
DPDK_VERSION="12.11"

# Download DPDK
echo "Downloading DPDK version $DPDK_VERSION..."
curl http://fast.dpdk.org/rel/dpdk-$DPDK_VERSION.tar.xz -o dpdk-$DPDK_VERSION.tar.xz -s -L --retry 5 --retry-delay 2 --retry-max-time 15 \
    || (echo "Failed to download DPDK" && exit 1)

# Extract the DPDK Archive
echo "Extracting DPDK..."
tar xf dpdk-$DPDK_VERSION.tar.xz

# Navigate to DPDK Directory
cd dpdk-$DPDK_VERSION

# Configure and Build DPDK
echo "Configuring and building DPDK..."
meson build
cd build
ninja
sudo ninja install

# Set Environment Variables
echo "Setting up environment variables..."
echo "export RTE_SDK=$(pwd)/.." >> ~/.bashrc
echo "export RTE_TARGET=x86_64-native-linuxapp-gcc" >> ~/.bashrc
source ~/.bashrc

# Done
echo "DPDK installation is complete."
