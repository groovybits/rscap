#!/bin/bash

# Exit immediately if a command exits with a non-zero status.
set -e

SUDO=""
if [ -f "/usr/bin/sudo" ]; then
    SUDO="/usr/bin/sudo"
fi
PKGMGR=""
if [ -f "/usr/bin/dnf" ]; then
    PKGMGR="/usr/bin/dnf"
elif [ -f "/usr/bin/yum" ]; then
    PKGMGR="/usr/bin/yum"
fi

# Function to get the distribution name from /etc/os-release
get_distro_name() {
    if [ -f /etc/os-release ]; then
        . /etc/os-release
        echo "$NAME"
    elif [ -f /etc/redhat-release ]; then
        # Older versions of CentOS might use /etc/redhat-release
        cat /etc/redhat-release
    else
        echo "Unknown"
    fi
}

OS=$(uname)
distro_name=""
distro_type=""

if [ "$OS" = "Linux" ]; then
    if [ "$PKGMGR" = "" ]; then
        echo "ERROR: No package manager found, dnf and yum don't exist!!!"
        exit 1
    fi

    distro_name=$(get_distro_name)
    distro_type="unknown"

    if [[ "$distro_name" == *"CentOS"* ]]; then
        echo "This is a CentOS system."
        distro_type="centos"
    elif [[ "$distro_name" == *"AlmaLinux"* ]]; then
        echo "This is an AlmaLinux system."
        distro_type="alma"
    else
        echo "This is a Linux system, but not CentOS or AlmaLinux."
    fi
fi

FEATURES=
if [ "$1" != "" ]; then
    FEATURES="--features $1"
fi

if [ "$BUILD" == "" ]; then
    echo "Using default build profile."
    BUILD=release-with-debug
else
    echo "Using build profile: $BUILD"
fi
PREFIX=/opt/rsprobe

LD_LIBRARY_PATH=$PREFIX/lib:$PREFIX/lib64:$LD_LIBRARY_PATH
export LD_LIBRARY_PATH

# Assuming your pkg-config files are in /opt/rsprobe/lib64/pkgconfig
PKG_CONFIG_PATH=/opt/rsprobe/lib64/pkgconfig:/opt/rsprobe/lib/pkgconfig:$PKG_CONFIG_PATH
export PKG_CONFIG_PATH

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
    export PKG_CONFIG_PATH=/opt/rsprobe/lib64/pkgconfig:/opt/rsprobe/lib/pkgconfig:$PKG_CONFIG_PATH
    if [ "$distro_type" = "centos" ]; then
        scl enable devtoolset-11 rh-python38 llvm-toolset-7.0 -- "$@"
    else
        "$@"
    fi
}

# Check for Rust and install if missing
if ! command -v cargo &> /dev/null
then
    echo "Installing Rust"
    install_rust
    source ~/.profile
fi

# CentOS 7 specific setup
if [ "$OS" = "Linux" ]; then
    export RUSTFLAGS="-C link-args=-Wl,-rpath,$PREFIX/lib:$PREFIX/lib64"

    if [ "$distro_type" = "centos" ]; then
        echo "CentOS detected."

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
    elif [ "$distro_type" = "alma" ]; then
        echo "Alma Linux Detected"
    fi

    # Build with SCL
    echo "Building project (CentOS 7)..."
    if [ "$BUILD" = "release" ]; then
        run_with_scl cargo build $FEATURES --release --quiet
    elif [ "$BUILD" = "release-with-debug" ]; then
        run_with_scl cargo build $FEATURES --profile=release-with-debug --quiet
    else
        run_with_scl cargo build $FEATURES --quiet
    fi
    # Add elif blocks here for other specific Linux distributions
elif [ "$OS" = "Darwin" ]; then
    #export RUSTFLAGS="-C link-args=-Wl,-rpath,$PREFIX/lib -Wl,-rpath,$PREFIX/lib64"

    echo "macOS detected."
    # Brew RPMs
    # check if brew binary exists
    if ! command -v brew &> /dev/null; then
        echo "Homebrew not found. Installing Homebrew..."
        /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
    fi
    # check if xcode-select is installed, if not install it
    # Check if Xcode Command Line Tools are installed
    xcode-select -p &> /dev/null

    if [ $? -eq 0 ]; then
        echo "Xcode Command Line Tools are installed."
    else
        echo "Xcode Command Line Tools are not installed."
        echo "Installing Xcode Command Line Tools..."
        xcode-select --install
        # Note: The user will need to continue the installation process manually if required.
    fi
    export CXXFLAGS="-stdlib=libc++"
    export LDFLAGS="-lc++"
    # macOS specific setup
    # Build on macOS
    echo "Building project (macOS)..."
    if [ "$BUILD" = "release" ]; then
        cargo build $FEATURES --release --quiet
    elif [ "$BUILD" == "release-with-debug" ]; then
        cargo build $FEATURES --profile=release-with-debug --quiet
    else
        cargo build $FEATURES --quiet
    fi
else
    export RUSTFLAGS="-C link-args=-Wl,-rpath,$PREFIX/lib:$PREFIX/lib64"
    echo "Generic Unix-like OS detected."
    # Generic Unix/Linux setup
    # Build for generic Unix/Linux
    echo "Building project..."
    if [ "$BUILD" = "release" ]; then
        cargo build $FEATURES --release --quiet
    elif [ "$BUILD" == "release-with-debug" ]; then
        cargo build $FEATURES --profile=release-with-debug --quiet
    else
        cargo build $FEATURES --quiet
    fi
fi

echo "Build completed successfully."
