#!/bin/bash
set -e

# Function to run a command within the SCL environment for CentOS
run_with_scl() {
    scl enable devtoolset-11 -- "$@"
}

# Define versions for dependencies and GStreamer
GLIB_VERSION=2.56.4
ORC_VERSION=0.4.31
GST_VERSION=1.20.0
LIBFFI_VERSION=3.3

# Define the installation prefix
PREFIX=/usr/local
export PATH=$PREFIX/bin:$PATH

# For pkg-config to find .pc files
export PKG_CONFIG_PATH=/usr/lib64/pkgconfig:/usr/local/lib64/pkgconfig:$PKG_CONFIG_PATH
export LD_LIBRARY_PATH=/usr/local/lib64:$LD_LIBRARY_PATH

# Ensure the system is up to date and has the basic build tools
sudo yum groupinstall -y "Development Tools"
sudo yum install -y bison flex python3 wget libffi-devel util-linux-devel util-linux libmount-devel

# Explicitly use cmake from /usr/local/bin for Meson configuration
echo "[binaries]" > meson-native-file.ini
echo "cmake = 'cmake3'" >> meson-native-file.ini

# Ensure Meson and Ninja are installed and use the correct Ninja
sudo pip3 install meson
sudo pip3 install ninja

echo "Installing GStreamer and essential dependencies..."

echo "Installing libffi..."
# Download, compile, and install libffi
if [ ! -f libffi-$LIBFFI_VERSION.tar.gz ]; then
    wget ftp://sourceware.org/pub/libffi/libffi-$LIBFFI_VERSION.tar.gz
fi
if [ ! -d libffi-$LIBFFI_VERSION ]; then
    tar xf libffi-$LIBFFI_VERSION.tar.gz
fi
cd libffi-$LIBFFI_VERSION
run_with_scl ./configure --prefix=$PREFIX
run_with_scl make
sudo make install
cd ..

echo "Installing ORC..."
# Download, compile, and install ORC
if [ ! -f orc-$ORC_VERSION.tar.xz ]; then
    wget https://gstreamer.freedesktop.org/src/orc/orc-$ORC_VERSION.tar.xz
fi
if [ ! -d orc-$ORC_VERSION ]; then
    tar xf orc-$ORC_VERSION.tar.xz
fi
cd orc-$ORC_VERSION
run_with_scl meson _build --prefix=$PREFIX --buildtype=release --native-file ../meson-native-file.ini
run_with_scl ninja -C _build
sudo /usr/local/bin/ninja -C _build install
cd ..

echo "Installing Gstreamer core..."
# Download, compile, and install GStreamer core
if [ ! -f gstreamer-$GST_VERSION.tar.xz ]; then
    wget https://gstreamer.freedesktop.org/src/gstreamer/gstreamer-$GST_VERSION.tar.xz
fi
if [ ! -d gstreamer-$GST_VERSION ]; then
    tar xf gstreamer-$GST_VERSION.tar.xz
fi
cd gstreamer-$GST_VERSION
run_with_scl meson _build --prefix=$PREFIX --buildtype=release --native-file ../meson-native-file.ini
run_with_scl ninja -C _build
sudo /usr/local/bin/ninja -C _build install
cd ..

echo "Installing Gstreamer base..."
# Download, compile, and install gst-plugins-base
if [ ! -f gst-plugins-base-$GST_VERSION.tar.xz ]; then
    wget https://gstreamer.freedesktop.org/src/gst-plugins-base/gst-plugins-base-$GST_VERSION.tar.xz
fi
if [ ! -d gst-plugins-base-$GST_VERSION ]; then
    tar xf gst-plugins-base-$GST_VERSION.tar.xz
fi
cd gst-plugins-base-$GST_VERSION
run_with_scl meson _build --prefix=$PREFIX --buildtype=release --native-file ../meson-native-file.ini
run_with_scl ninja -C _build
sudo /usr/local/bin/ninja -C _build install
cd ..

# Verify GStreamer installation
echo "Verifying GStreamer installation..."
gst-launch-1.0 --version

echo "GStreamer and essential dependencies installed."
