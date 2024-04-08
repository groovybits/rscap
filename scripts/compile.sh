#!/bin/bash

# Exit immediately if a command exits with a non-zero status.
set -e

FEATURES=
if [ "$1" != "" ]; then
    FEATURES="--features $1"
fi

# Function to prompt for installation
prompt_install() {
    while true; do
        read -p "Do you wish to install $1? [Y/n] " yn
        case $yn in
            [Yy]* ) return 0;;
            [Nn]* ) return 1;;
            * ) echo "Please answer yes or no.";;
        esac
    done
}

# Function to install Rust
install_rust() {
    if prompt_install "Rust"; then
        echo "Installing Rust..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
        source $HOME/.cargo/env
    else
        echo "Rust installation skipped. Exiting."
        exit 1
    fi
}

# Function to run a command within the SCL environment for CentOS
run_with_scl() {
    export PKG_CONFIG_PATH=/usr/local/lib64/pkgconfig:/usr/lib64/pkgconfig:$PKG_CONFIG_PATH
    scl enable devtoolset-11 -- "$@"
}

# Detect the operating system
OS="$(uname -s)"
echo "Detected OS: $OS"

# Check for Rust and install if missing
if ! command -v cargo &> /dev/null
then
    install_rust
fi

# CentOS 7 specific setup
if [ "$OS" = "Linux" ]; then
    if [ -f /etc/centos-release ]; then
        . /etc/os-release
        if [ "$VERSION_ID" = "7" ]; then
            echo "CentOS 7 detected."

            # Check for SCL and install if missing
            if ! command -v scl &> /dev/null; then
                if prompt_install "Software Collections (SCL)"; then
                    echo "Installing Software Collections..."
                    sudo yum install centos-release-scl -y
                    sudo yum install devtoolset-11 -y
                else
                    echo "SCL installation skipped. Exiting."
                    exit 1
                fi
            fi

            # Build with SCL
            echo "Building project (CentOS 7)..."
            run_with_scl cargo build $FEATURES
            run_with_scl cargo build $FEATURES --release
            run_with_scl cargo build $FEATURES --profile=release-with-debug
        fi
    fi
    # Add elif blocks here for other specific Linux distributions
elif [ "$OS" = "Darwin" ]; then
    echo "macOS detected."
    # macOS specific setup
    # Build on macOS
    echo "Building project (macOS)..."
    cargo build $FEATURES
    cargo build $FEATURES --release
    cargo build $FEATURES --profile=release-with-debug
else
    echo "Generic Unix-like OS detected."
    # Generic Unix/Linux setup
    # Build for generic Unix/Linux
    echo "Building project..."
    cargo build $FEATURES
    cargo build $FEATURES --release
    cargo build $FEATURES --profile=release-with-debug
fi

echo "Build completed successfully."
