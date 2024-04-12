Name:           rscap
Version:        0.5.27
Release:        1%{?dist}
Summary:        RsCap and GStreamer with essential dependencies

License:        MIT
URL:            https://github.com/groovybits/rscap

BuildRequires:  gcc, gcc-c++, make, python3, wget, libffi-devel, util-linux, libmount-devel, bison, flex, git, cmake3
Requires:       glib2, orc

%description
RsCap is a rust based pcap for MpegTS and SMPTE2110 UDP/TCP Broadcast Feeds

%prep

%build
set -e

# Function to run a command within the SCL environment for CentOS
run_with_scl() {
    scl enable devtoolset-11 -- "$@"
}

BUILD_DIR=%{_builddir}
mkdir -p $BUILD_DIR
cd $BUILD_DIR

# Define versions for dependencies of GStreamer
GLIB_VERSION=2.56.4
ORC_VERSION=0.4.31
GST_VERSION=1.20.0
LIBFFI_VERSION=3.3
NASM_VERSION=2.15.05
FFMPEG_VERSION=5.1.4
RUST_VERSION=1.77.1
RSCAP_VERSION=0.5.27

# Define the installation prefix
PREFIX=$BUILD_DIR/opt/rscap

# Ensure the system has the basic build tools
yum groupinstall -y "Development Tools"
yum install -y bison flex python3 wget libffi-devel util-linux libmount-devel

# Define the PATH to include the Rust binaries

export PATH=$PREFIX/bin:$PATH

# For pkg-config to find .pc files
export PKG_CONFIG_PATH=$PREFIX/lib64/pkgconfig:$PREFIX/lib/pkgconfig:/usr/lib64/pkgconfig:$PKG_CONFIG_PATH
export LD_LIBRARY_PATH=$PREFIX/lib64:$PREFIX/lib:$LD_LIBRARY_PATH

# Explicitly use cmake3 for Meson configuration
echo "[binaries]" > meson-native-file.ini
echo "cmake = 'cmake3'" >> meson-native-file.ini
CWD=$(pwd)
MESON_NATIVE_FILE=$CWD/meson-native-file.ini

# Install Meson and Ninja
pip3 install meson
pip3 install ninja

echo "------------------------------------------------------------"
echo "Buidling and installing GStreamer with essential dependencies..."
echo "------------------------------------------------------------"

# Download and extract Rust locally
curl --proto '=https' --tlsv1.2 -sSf https://static.rust-lang.org/dist/rust-$RUST_VERSION-x86_64-unknown-linux-gnu.tar.gz | tar -xz
mv rust-$RUST_VERSION-x86_64-unknown-linux-gnu rust
cd rust
./install.sh --prefix=$PREFIX --without=rust-docs
cd ..
rm -rf rust

# Install libFFI
echo "---"
echo "Installing libffi..."
echo "---"
# Download, compile, and install libffi
wget ftp://sourceware.org/pub/libffi/libffi-$LIBFFI_VERSION.tar.gz
tar xf libffi-$LIBFFI_VERSION.tar.gz
cd libffi-$LIBFFI_VERSION
run_with_scl ./configure --prefix=$PREFIX
run_with_scl make
make install
cd ..
rm -rf libffi-$LIBFFI_VERSION
rm -rf libffi-$LIBFFI_VERSION.tar.gz

# Install ORC
echo "---"
echo "Installing ORC..."
echo "---"
# Download, compile, and install ORC
wget https://gstreamer.freedesktop.org/src/orc/orc-$ORC_VERSION.tar.xz
tar xf orc-$ORC_VERSION.tar.xz
cd orc-$ORC_VERSION
run_with_scl meson _build --prefix=$PREFIX --buildtype=release --native-file $MESON_NATIVE_FILE
run_with_scl ninja -C _build
ninja -C _build install
cd ..
rm -rf orc-$ORC_VERSION
rm -rf orc-$ORC_VERSION.tar.xz

# Install Gstreamer core
echo "---"
echo "Installing Gstreamer core..."
echo "---"
# Download, compile, and install GStreamer core
wget https://gstreamer.freedesktop.org/src/gstreamer/gstreamer-$GST_VERSION.tar.xz
tar xf gstreamer-$GST_VERSION.tar.xz
cd gstreamer-$GST_VERSION
run_with_scl meson _build --prefix=$PREFIX --buildtype=release --native-file $MESON_NATIVE_FILE
run_with_scl ninja -C _build
ninja -C _build install
cd ..
rm -rf gstreamer-$GST_VERSION
rm -rf gstreamer-$GST_VERSION.tar.xz

# Install GStreamer base plugins
echo "---"
echo "Installing Gstreamer base..."
echo "---"
# Download, compile, and install gst-plugins-base
wget https://gstreamer.freedesktop.org/src/gst-plugins-base/gst-plugins-base-$GST_VERSION.tar.xz
tar xf gst-plugins-base-$GST_VERSION.tar.xz
cd gst-plugins-base-$GST_VERSION
run_with_scl meson _build --prefix=$PREFIX --buildtype=release --native-file $MESON_NATIVE_FILE
run_with_scl ninja -C _build
ninja -C _build install
cd ..
rm -rf gst-plugins-base-$GST_VERSION
rm -rf gst-plugins-base-$GST_VERSION.tar.xz

# Install GStreamer bad plugins (includes tsdemux)
echo "---"
echo "Installing Gstreamer bad plugins..."
echo "---"
wget https://gstreamer.freedesktop.org/src/gst-plugins-bad/gst-plugins-bad-$GST_VERSION.tar.xz
tar xf gst-plugins-bad-$GST_VERSION.tar.xz
cd gst-plugins-bad-$GST_VERSION
run_with_scl meson _build --prefix=$PREFIX --buildtype=release --native-file $MESON_NATIVE_FILE
run_with_scl ninja -C _build
ninja -C _build install
cd ..
rm -rf gst-plugins-bad-$GST_VERSION
rm -rf gst-plugins-bad-$GST_VERSION.tar.xz

echo "---"
echo "Downloading and compiling NASM (Netwide Assembler)..."
echo "---"
# Download and compile NASM
wget https://www.nasm.us/pub/nasm/releasebuilds/$NASM_VERSION/nasm-$NASM_VERSION.tar.gz
tar -xzf nasm-$NASM_VERSION.tar.gz
cd nasm-$NASM_VERSION
./autogen.sh
./configure --prefix=$PREFIX
make
make install
cd ..
rm -rf nasm-$NASM_VERSION
rm -rf nasm-$NASM_VERSION.tar.gz

# libx264
echo "---"
echo "Cloning and compiling libx264..."
echo "---"
git clone https://code.videolan.org/videolan/x264.git
cd x264
run_with_scl ./configure --prefix=$PREFIX --enable-shared --enable-static --enable-pic
run_with_scl make
make install
ldconfig
cd ..
rm -rf x264

# libx265
echo "---"
echo "Cloning and compiling x265..."
echo "---"
git clone https://github.com/videolan/x265.git
cd x265
mkdir -p build
cd build
run_with_scl cmake3 -G "Unix Makefiles" -DCMAKE_INSTALL_PREFIX=$PREFIX -DENABLE_SHARED:bool=on ../source
run_with_scl make
make install
ldconfig
cd ../..
rm -rf x265

# FFmpeg
echo "---"
echo "Downloading and compiling FFmpeg..."
echo "---"
wget http://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.bz2
tar xf ffmpeg-$FFMPEG_VERSION.tar.bz2
cd ffmpeg-$FFMPEG_VERSION
run_with_scl ./configure --prefix=$PREFIX \
    --enable-shared --enable-static \
    --enable-pic --enable-gpl --enable-libx264 \
    --enable-libx265 \
    --extra-cflags="-I$PREFIX/include" --extra-ldflags="-L$PREFIX/lib"
run_with_scl make
make install
ldconfig
cd ..
rm -rf ffmpeg-$FFMPEG_VERSION
rm -rf ffmpeg-$FFMPEG_VERSION.tar.bz2

# GStreamer libav plugins
echo "---"
echo "Installing Gstreamer libav plugins..."
echo "---"
PWD=$(pwd)
echo "PWD: $PWD"
wget https://gstreamer.freedesktop.org/src/gst-libav/gst-libav-$GST_VERSION.tar.xz
tar xf gst-libav-$GST_VERSION.tar.xz
cd gst-libav-$GST_VERSION
run_with_scl meson _build --prefix=$PREFIX --buildtype=release --native-file $MESON_NATIVE_FILE --pkg-config-path=$PKG_CONFIG_PATH
run_with_scl ninja -C _build
ninja -C _build install
cd ..
rm -rf gst-libav-$GST_VERSION
rm -rf gst-libav-$GST_VERSION.tar.xz

# GStreamer good plugins
echo "---"
echo "Installing GStreamer good plugins..."
echo "---"
wget https://gstreamer.freedesktop.org/src/gst-plugins-good/gst-plugins-good-$GST_VERSION.tar.xz
tar xf gst-plugins-good-$GST_VERSION.tar.xz
cd gst-plugins-good-$GST_VERSION
run_with_scl meson _build --prefix=$PREFIX --buildtype=release --native-file $MESON_NATIVE_FILE
run_with_scl ninja -C _build
ninja -C _build install
cd ..
rm -rf gst-plugins-good-$GST_VERSION
rm -rf gst-plugins-good-$GST_VERSION.tar.xz

# Verify GStreamer installation
echo "------------------------------------------------------------"
echo "Verifying GStreamer installation..."
echo "------------------------------------------------------------"
$PREFIX/bin/gst-launch-1.0 --version

echo "------------------------------------------------------------"
echo "GStreamer and essential dependencies installed."
echo "------------------------------------------------------------"

echo "------------------------------------------------------------"
echo "Building RsCap..."
echo "------------------------------------------------------------"

# Set RUSTFLAGS for RPATH
export RUSTFLAGS="-C link-args=-Wl,-rpath,/opt/rscap/lib:/opt/rscap/lib64"

# Set environment variables for Rust
export CARGO_HOME=$PREFIX/cargo
export CARGO=$PREFIX/bin/cargo
export RUSTC=$PREFIX/bin/rustc

# Build RsCap
echo "Building RsCap..."

# Clone RsCap repository and checkout the specific tag
git clone https://github.com/groovybits/rscap.git
cd rscap
git checkout $RSCAP_VERSION

## Build RsCap
run_with_scl cargo build --features gst --release
cd ..

# Remove existing binaries, we do not need them
rm -rf $PREFIX/cargo
rm -rf $PREFIX/bin/*
rm -rf $PREFIX/share
rm -rf $PREFIX/etc
rm -rf $PREFIX/lib/rustlib
rm -rf $PREFIX/lib/*.a
rm -rf $PREFIX/lib/pkgconfig
rm -rf $PREFIX/lib64/*.a
rm -rf $PREFIX/lib64/pkgconfig
rm -rf $PREFIX/include

# Copy RsCap binaries to the installation directory
cp rscap/target/release/probe $PREFIX/bin/
cp rscap/target/release/monitor $PREFIX/bin/

# cleanup rscap
rm -rf rscap

echo "------------------------------------------------------------"
echo "Done building RsCap and all dependencies."
echo "------------------------------------------------------------"

%install
echo "---"
echo "Installing RsCap..."
echo "---"

# Remove previous buildroot
rm -rf %{buildroot}

# Create the necessary directories in the RPM build root
mkdir -p %{buildroot}/opt/rscap/bin
mkdir -p %{buildroot}/opt/rscap/lib
mkdir -p %{buildroot}/opt/rscap/lib64

# Copy only the desired directories to the RPM build root
cp -R %{_builddir}/opt/rscap/bin/* %{buildroot}/opt/rscap/bin/
cp -R %{_builddir}/opt/rscap/lib/* %{buildroot}/opt/rscap/lib/
cp -R %{_builddir}/opt/rscap/lib64/* %{buildroot}/opt/rscap/lib64/

echo "------------------------------------------------------------"
echo "Finished installing RsCap."
echo "------------------------------------------------------------"

%files
/opt/rscap/bin/*
/opt/rscap/lib/*
/opt/rscap/lib64/*

%changelog
* Mon Apr 08 2024 Chris Kennedy <chris@rscap.com>
- Initial RPM release
