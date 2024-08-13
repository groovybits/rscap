#!/bin/bash
#set -v
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

    ## Setup Yum Repos
    $SUDO $PKGMGR install -yq epel-release
    if [ "$distro_type" = "alma" ]; then
        $SUDO $PKGMGR config-manager --set-enabled powertools
        $SUDO $PKGMGR install -yq almalinux-release-synergy
    fi
    $SUDO $PKGMGR install -yq which

    ## Standard Development tools
    $SUDO $PKGMGR groupinstall -yq "Development Tools"
    if [ "$distro_type" = "alma" ]; then
        $SUDO $PKGMGR install -yq cmake
    else
        $SUDO $PKGMGR install -yq cmake3
    fi
    $SUDO $PKGMGR install -yq zlib-devel openssl-devel
    $SUDO $PKGMGR install -yq screen

    if [ "$distro_type" = "alma" ]; then
        ## Alma Release Synergy GRPC and Protobuf
        $SUDO $PKGMGR install -yq grpc grpc-devel
        $SUDO $PKGMGR install -yq protobuf-devel protobuf
        $SUDO $PKGMGR install -yq grpc-plugins
    fi
fi

if [ "$CLEAN" == "" ]; then
    CLEAN=1
else
    CLEAN=$CLEAN
fi

# Function to run a command within the SCL environment for CentOS
run_with_scl() {
    OS="$(uname -s)"
    if [ "$OS" = "Linux" -a "$distro_type" = "centos" ]; then
        scl enable devtoolset-11 rh-python38 -- "$@"
    else
        "$@"
    fi
}

run_with_scl_llvm() {
    OS="$(uname -s)"
    if [ "$OS" = "Linux" -a "$distro_type" = "centos" ]; then
        scl enable devtoolset-11 llvm-toolset-7.0 -- "$@"
    else
        "$@"
    fi
}

BUILD_DIR=$(pwd)/build
if [ ! -d $BUILD_DIR ]; then
    mkdir -p $BUILD_DIR
else
    if [ "$CLEAN" == "1" ]; then
        rm -rf build/*
    fi
fi
cd $BUILD_DIR

# Define versions for dependencies and GStreamer
GLIB_MAJOR_VERSION=2.64
GLIB_VERSION=${GLIB_MAJOR_VERSION}.6
NASM_VERSION=2.15.05
FFMPEG_VERSION=6.1.1
LIBZVBI_VERSION=0.2.42

# Define the installation prefix
export PREFIX=/opt/rsprobe
export PATH=$PREFIX/bin:$PATH

USER=$(whoami)
if [ "$USER" == "root" ]; then
    SUDO=
fi
echo "User $USER executing script"
if [ ! -d "$PREFIX" ]; then
    $SUDO mkdir -p $PREFIX
fi
$SUDO chown -R $USER $PREFIX
# Ensure necessary tools are installed

# For pkg-config to find .pc files
export PKG_CONFIG_PATH=$PREFIX/lib64/pkgconfig:${PREFIX}/lib/pkgconfig:$PKG_CONFIG_PATH
export LD_LIBRARY_PATH=$PREFIX/lib64:${PREFIX}/lib:$LD_LIBRARY_PATH

export PATH=$PREFIX/bin:$PATH
CPUS=1
if [ "$OS" = "Linux" ]; then
    CPUS=$(nproc)
elif [ "$OS" == "Darwin" ]; then
    CPUS=$(sysctl -n hw.ncpu)
fi

if [ "$OS" = "Linux" ]; then
    # Ensure the system is up to date and has the basic build tools
    if [ "$distro_type" = "alma" ]; then
        if [ "$GST" = "true" ]; then
            $SUDO $PKGMGR install -yq python3 wget python3.12 python3.12-pip
            $SUDO pip3.12 install meson
            $SUDO pip3.12 install ninja
            $SUDO pip3.12 install numpy
        fi
    else
        $SUDO $PKGMGR groupinstall -yq "Development Tools"
        $SUDO $PKGMGR install -yq bison flex python3 wget libffi-devel util-linux \
            libmount-devel libxml2-devel glib2-devel cairo-devel \
            ladspa-devel pango-devel cairo-gobject-devel cairo-gobject
        $SUDO $PKGMGR install -yq centos-release-scl-rh epel-release
        $SUDO $PKGMGR install -yq yum-utils
        $SUDO yum-config-manager --disable epel
        $SUDO $PKGMGR install --enablerepo=epel* -yq zvbi-devel cmake3
        $SUDO $PKGMGR install -yq git
        $SUDO $PKGMGR install -yq libstdc++-devel
        $SUDO $PKGMGR install -yq llvm-toolset-7.0-llvm-devel llvm-toolset-7.0-clang
        $SUDO $PKGMGR install -yq rh-python38 rh-python38-python-pip

        $SUDO pip3 install meson
        $SUDO pip3 install ninja
        run_with_scl $SUDO pip3.8 install meson
        run_with_scl $SUDO pip3.8 install ninja
    fi
fi

# Ensure Meson and Ninja are installed and use the correct Ninja
if [ "$OS" = "Darwin" ]; then
    brew install meson
    brew install ninja
    brew install pkg-config
    brew install bison
    brew install wget
    brew install llvm
    brew install cmake
    export PATH="/usr/local/opt/bison/bin:$PATH"
fi

if [ "$OS" == "Linux" ]; then
    export RUSTFLAGS="-C link-args=-Wl,-rpath,$PREFIX/lib:$PREFIX/lib64"
else
    export RUSTFLAGS="-C link-args=-Wl,-rpath,$PREFIX/lib"
fi

# RsCap installation
echo
echo "Changing to RsCap directory... 'cd ../'"
cd ..
echo "------------------------------------------------------------"
echo "Cleaning RsCap..."
cargo clean --quiet
CUR_DIR=$(pwd)
echo "Building RsCap in $CUR_DIR ..."
export BUILD_TYPE=release
BUILD=$BUILD_TYPE ./scripts/compile.sh $GST_FEATURE

# Copy RsCap binaries to the installation directory
echo "Copying RsCap binaries to the installation directory..."
mkdir -p $PREFIX/bin
cp -f target/$BUILD_TYPE/probe $PREFIX/bin/
cp -f scripts/probe.sh $PREFIX/bin/
cp -f scripts/setup_env.sh $PREFIX/bin/

echo "------------------------------------------------------------"
echo "RsCap installation completed."
ls -altr $PREFIX/bin/probe

probe -V

