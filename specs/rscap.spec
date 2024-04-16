Name:           rscap
Version:        0.5.45
Release:        1%{?dist}
Summary:        RsCap and GStreamer with essential dependencies
License:        MIT
URL:            https://github.com/groovybits/rscap
BuildRequires:  epel-release, centos-release-scl-rh, gcc, gcc-c++, make, python3, wget, libffi-devel, util-linux, libmount-devel, bison, flex, git, cmake3, libxml2-devel, pango-devel, cairo-devel, zvbi-devel, ladspa-devel, cairo-gobject-devel, cairo-gobject, rh-python38, rh-python38-python-pip
Requires:       orc, libffi
Prefix:        /opt/rscap

%description
Capture probe with ZMQ connected monitor for analyzing MpegTS UDP Streams and sending status to Kafka with thumbnails and metadata information.

# prepare the source code
%prep

# Build the source code
%build

# Function to run a command within the SCL environment for CentOS
run_with_scl() {
    scl enable rh-python38 devtoolset-11 -- "$@"
}

# Define versions for dependencies including GStreamer
RSCAP_VERSION=%{version}
RUST_VERSION=1.77.1
NASM_VERSION=2.15.05
FFMPEG_VERSION=6.1.1
GST_VERSION=1.24.2
GST_PLUGINS_RS_VERSION=gstreamer-$GST_VERSION
GLIB_MAJOR_VERSION=2.64
GLIB_VERSION=$GLIB_MAJOR_VERSION.6
ORC_VERSION=0.4.31

# Create builddir
mkdir -p %{_builddir}%{prefix}
cd %{_builddir}

# Define the PATH to include the Rust binaries
export PATH=%{_builddir}%{prefix}/bin:$PATH

# For pkg-config to find .pc files
export PKG_CONFIG_PATH=%{_builddir}%{prefix}/lib64/pkgconfig:%{_builddir}%{prefix}/lib/pkgconfig:$PKG_CONFIG_PATH
export LD_LIBRARY_PATH=%{_builddir}%{prefix}/lib64:%{_builddir}%{prefix}/lib:$LD_LIBRARY_PATH

# Set RUSTFLAGS for RPATH
export RUSTFLAGS="-C link-args=-Wl,-rpath,%{prefix}/lib:%{prefix}/lib64"

# Explicitly use cmake3 for Meson configuration
echo "[binaries]" > %{_builddir}/meson-native-file.ini
echo "cmake = 'cmake3'" >> %{_builddir}/meson-native-file.ini
MESON_NATIVE_FILE=%{_builddir}/meson-native-file.ini

# Install Meson and Ninja
run_with_scl pip3.8 install meson
run_with_scl pip3.8 install ninja

echo "------------------------------------------------------------"
echo "Buidling and installing GStreamer with essential dependencies..."
echo "------------------------------------------------------------"

# Download and build glib
wget https://download.gnome.org/sources/glib/$GLIB_MAJOR_VERSION/glib-$GLIB_VERSION.tar.xz
tar xf glib-$GLIB_VERSION.tar.xz
cd glib-$GLIB_VERSION
run_with_scl meson _build --prefix=%{_builddir}%{prefix} --buildtype=release --native-file $MESON_NATIVE_FILE --pkg-config-path=$PKG_CONFIG_PATH
run_with_scl ninja -C _build
run_with_scl ninja -C _build install
cd ..
rm -rf glib-$GLIB_VERSION.tar.xz
rm -rf cd glib-$GLIB_VERSION

# Download and extract Rust locally
curl --proto '=https' --tlsv1.2 -sSf https://static.rust-lang.org/dist/rust-$RUST_VERSION-x86_64-unknown-linux-gnu.tar.gz | tar -xz
mv rust-$RUST_VERSION-x86_64-unknown-linux-gnu rust
cd rust
./install.sh --prefix=%{_builddir}%{prefix} --without=rust-docs
cd ..
rm -rf rust

# Install ORC
echo "---"
echo "Installing ORC..."
echo "---"
# Download, compile, and install ORC
wget https://gstreamer.freedesktop.org/src/orc/orc-$ORC_VERSION.tar.xz
tar xf orc-$ORC_VERSION.tar.xz
cd orc-$ORC_VERSION
run_with_scl meson _build --prefix=%{_builddir}%{prefix} --buildtype=release --native-file $MESON_NATIVE_FILE --pkg-config-path=$PKG_CONFIG_PATH
run_with_scl ninja -C _build
run_with_scl ninja -C _build install
cd ..
rm -rf orc-$ORC_VERSION
rm -f orc-$ORC_VERSION.tar.xz

echo "---"
echo "Downloading and compiling NASM (Netwide Assembler)..."
echo "---"
# Download and compile NASM
wget https://www.nasm.us/pub/nasm/releasebuilds/$NASM_VERSION/nasm-$NASM_VERSION.tar.gz
tar -xzf nasm-$NASM_VERSION.tar.gz
cd nasm-$NASM_VERSION
./autogen.sh
./configure --prefix=%{_builddir}%{prefix}
make
make install
cd ..
rm -rf nasm-$NASM_VERSION
rm -f nasm-$NASM_VERSION.tar.gz

# libx264
echo "---"
echo "Cloning and compiling libx264..."
echo "---"
git clone https://code.videolan.org/videolan/x264.git
cd x264
run_with_scl ./configure --prefix=%{_builddir}%{prefix} --enable-shared --enable-static --enable-pic --libdir=%{_builddir}%{prefix}/lib64 --includedir=%{_builddir}%{prefix}/include --extra-ldflags="-L%{_builddir}%{prefix}/lib64"
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
run_with_scl cmake3 -G "Unix Makefiles" -DCMAKE_INSTALL_PREFIX=%{_builddir}%{prefix} -DCMAKE_INSTALL_LIBDIR=lib64 -DCMAKE_BUILD_TYPE=Release -DENABLE_SHARED:bool=on --libdir=%{_builddir}%{prefix}/lib64 --includedir=%{_buildidr}%{prefix}/include --extra-ldflags="-L%{_builddir}%{prefix}/lib64" ../source
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
run_with_scl ./configure --prefix=%{_builddir}%{prefix} \
    --enable-shared --enable-static \
    --enable-pic --enable-gpl --enable-libx264 \
    --enable-libx265 --enable-libzvbi \
    --extra-cflags="-I%{_builddir}%{prefix}/include" --extra-ldflags="-L%{_builddir}%{prefix}/lib" \
    --libdir=%{_builddir}%{prefix}/lib64
run_with_scl make
make install
ldconfig
cd ..
rm -rf ffmpeg-$FFMPEG_VERSION
rm -f ffmpeg-$FFMPEG_VERSION.tar.bz2

# Install Gstreamer core
echo "---"
echo "Installing Gstreamer core..."
echo "---"
# Download, compile, and install GStreamer core
wget https://gstreamer.freedesktop.org/src/gstreamer/gstreamer-$GST_VERSION.tar.xz
tar xf gstreamer-$GST_VERSION.tar.xz
cd gstreamer-$GST_VERSION
run_with_scl meson _build --prefix=%{_builddir}%{prefix} --buildtype=release --native-file $MESON_NATIVE_FILE --pkg-config-path=$PKG_CONFIG_PATH
run_with_scl ninja -C _build
run_with_scl ninja -C _build install
cd ..
rm -rf gstreamer-$GST_VERSION
rm -f gstreamer-$GST_VERSION.tar.xz

# Install GStreamer base plugins
echo "---"
echo "Installing Gstreamer base..."
echo "---"
# Download, compile, and install gst-plugins-base
wget https://gstreamer.freedesktop.org/src/gst-plugins-base/gst-plugins-base-$GST_VERSION.tar.xz
tar xf gst-plugins-base-$GST_VERSION.tar.xz
cd gst-plugins-base-$GST_VERSION
run_with_scl meson _build --prefix=%{_builddir}%{prefix} --buildtype=release --native-file $MESON_NATIVE_FILE --pkg-config-path=$PKG_CONFIG_PATH
run_with_scl ninja -C _build
run_with_scl ninja -C _build install
cd ..
rm -rf gst-plugins-base-$GST_VERSION
rm -f gst-plugins-base-$GST_VERSION.tar.xz

# Install GStreamer bad plugins (includes tsdemux)
echo "---"
echo "Installing Gstreamer bad plugins..."
echo "---"
wget https://gstreamer.freedesktop.org/src/gst-plugins-bad/gst-plugins-bad-$GST_VERSION.tar.xz
tar xf gst-plugins-bad-$GST_VERSION.tar.xz
cd gst-plugins-bad-$GST_VERSION
run_with_scl meson _build --prefix=%{_builddir}%{prefix} --buildtype=release --native-file $MESON_NATIVE_FILE --pkg-config-path=$PKG_CONFIG_PATH
run_with_scl ninja -C _build
run_with_scl ninja -C _build install
cd ..
rm -rf gst-plugins-bad-$GST_VERSION
rm -f gst-plugins-bad-$GST_VERSION.tar.xz

# GStreamer libav plugins
echo "---"
echo "Installing Gstreamer libav plugins..."
echo "---"
PWD=$(pwd)
echo "PWD: $PWD"
wget https://gstreamer.freedesktop.org/src/gst-libav/gst-libav-$GST_VERSION.tar.xz
tar xf gst-libav-$GST_VERSION.tar.xz
cd gst-libav-$GST_VERSION
run_with_scl meson _build --prefix=%{_builddir}%{prefix} --buildtype=release --native-file $MESON_NATIVE_FILE --pkg-config-path=$PKG_CONFIG_PATH
run_with_scl ninja -C _build
run_with_scl ninja -C _build install
cd ..
rm -rf gst-libav-$GST_VERSION
rm -f gst-libav-$GST_VERSION.tar.xz

# GStreamer good plugins
echo "---"
echo "Installing GStreamer good plugins..."
echo "---"
wget https://gstreamer.freedesktop.org/src/gst-plugins-good/gst-plugins-good-$GST_VERSION.tar.xz
tar xf gst-plugins-good-$GST_VERSION.tar.xz
cd gst-plugins-good-$GST_VERSION
run_with_scl meson _build --prefix=%{_builddir}%{prefix} --buildtype=release --native-file $MESON_NATIVE_FILE --pkg-config-path=$PKG_CONFIG_PATH
run_with_scl ninja -C _build
run_with_scl ninja -C _build install
cd ..
rm -rf gst-plugins-good-$GST_VERSION
rm -rf gst-plugins-good-$GST_VERSION.tar.xz

# Set environment variables for Rust
export CARGO_HOME=%{_builddir}%{prefix}/cargo
export CARGO=%{_builddir}%{prefix}/bin/cargo
export RUSTC=%{_builddir}%{prefix}/bin/rustc

# GStreamer Rust plugins
run_with_scl cargo install cargo-c --root=%{_builddir}%{prefix}

rm -rf gst-plugin-rs
git clone https://github.com/sdroege/gst-plugin-rs.git
cd gst-plugin-rs
git checkout $GST_PLUGINS_RS_VERSION

# Closed Caption
run_with_scl cargo cbuild --release --package gst-plugin-closedcaption
run_with_scl cargo cinstall --release --package gst-plugin-closedcaption --prefix=%{_builddir}%{prefix} --libdir=%{_builddir}%{prefix}/lib64

# Audio
run_with_scl cargo cbuild --release --package gst-plugin-audiofx
run_with_scl cargo cinstall --release --package gst-plugin-audiofx --prefix=%{_builddir}%{prefix} --libdir=%{_builddir}%{prefix}/lib64

# Video
run_with_scl cargo cbuild --release --package gst-plugin-videofx
run_with_scl cargo cinstall --release --package gst-plugin-videofx --prefix=%{_builddir}%{prefix} --libdir=%{_builddir}%{prefix}/lib64

cd ..
rm -rf gst-plugin-rs

# Verify GStreamer installation
echo "------------------------------------------------------------"
echo "Verifying GStreamer installation..."
echo "------------------------------------------------------------"
%{_builddir}%{prefix}/bin/gst-launch-1.0 --version

echo "------------------------------------------------------------"
echo "GStreamer and essential dependencies installed."
echo "------------------------------------------------------------"

# Build RsCap
echo "------------------------------------------------------------"
echo "Building RsCap..."
echo "------------------------------------------------------------"

# Clone RsCap repository and checkout the specific tag
rm -rf rscap
git clone https://github.com/groovybits/rscap.git
cd rscap

## Build RsCap
git checkout $RSCAP_VERSION
run_with_scl cargo build --features gst --release

# Copy RsCap binaries to the installation directory
cp target/release/probe %{_builddir}%{prefix}/bin/
cp target/release/monitor %{_builddir}%{prefix}/bin/
cp scripts/monitor.sh %{_builddir}%{prefix}/bin
cp scripts/probe.sh %{_builddir}%{prefix}/bin
cp scripts/setup_env.sh %{_builddir}%{prefix}/bin

cd ..

# cleanup rscap
rm -rf rscap

# Remove unnecessary build dependencies
rm -rf %{_builddir}%{prefix}/cargo
rm -rf %{_builddir}%{prefix}/share
rm -rf %{_builddir}%{prefix}/lib/rustlib
rm -rf %{_builddir}%{prefix}/lib/*.a
rm -rf %{_builddir}%{prefix}/lib64/*.a
rm -rf %{_builddir}%{prefix}/libexec
rm -rf %{_builddir}%{prefix}/etc
rm -f %{_builddir}%{prefix}/lib/librustc_driver-*
rm -f %{_builddir}%{prefix}/lib/libstd-*
rm -f %{_builddir}%{prefix}/lib/libLLVM-*
rm -f %{_builddir}%{prefix}/bin/gtester
rm -f %{_builddir}%{prefix}/bin/gobject-query
rm -f %{_builddir}%{prefix}/bin/gio
rm -f %{_builddir}%{prefix}/bin/gresource
rm -f %{_builddir}%{prefix}/bin/gio-querymodules
rm -f %{_builddir}%{prefix}/bin/glib-compile-schemas
rm -f %{_builddir}%{prefix}/bin/glib-compile-resources
rm -f %{_builddir}%{prefix}/bin/gsettings
rm -f %{_builddir}%{prefix}/bin/gdbus
rm -f %{_builddir}%{prefix}/bin/gapplication
rm -f %{_builddir}%{prefix}/bin/gtester-report
rm -f %{_builddir}%{prefix}/bin/glib-genmarshal
rm -f %{_builddir}%{prefix}/bin/glib-mkenums
rm -f %{_builddir}%{prefix}/bin/gdbus-codegen
rm -f %{_builddir}%{prefix}/bin/glib-gettextize
rm -f %{_builddir}%{prefix}/bin/rust-gdb
rm -f %{_builddir}%{prefix}/bin/rust-gdbgui
rm -f %{_builddir}%{prefix}/bin/rust-lldb
rm -f %{_builddir}%{prefix}/bin/rustc
rm -f %{_builddir}%{prefix}/bin/rustdoc
rm -f %{_builddir}%{prefix}/bin/rust-demangler
rm -f %{_builddir}%{prefix}/bin/cargo*
rm -f %{_builddir}%{prefix}/bin/cargo-fmt
rm -f %{_builddir}%{prefix}/bin/rustfmt
rm -f %{_builddir}%{prefix}/bin/rls
rm -f %{_builddir}%{prefix}/bin/rust-analyzer
rm -f %{_builddir}%{prefix}/bin/cargo-clippy
rm -f %{_builddir}%{prefix}/bin/clippy-driver
rm -f %{_builddir}%{prefix}/bin/orcc
rm -f %{_builddir}%{prefix}/bin/orc-bugreport
rm -f %{_builddir}%{prefix}/bin/nasm
rm -f %{_builddir}%{prefix}/bin/ndisasm

# Uncomment for production slim build without development files
rm -rf %{_builddir}%{prefix}/lib/pkgconfig
rm -rf %{_builddir}%{prefix}/lib64/pkgconfig
rm -rf %{_builddir}%{prefix}/include

echo "------------------------------------------------------------"
echo "Done building RsCap and all dependencies."
echo "------------------------------------------------------------"

# Installation script
%install
# Remove previous buildroot
rm -rf %{buildroot}

# Create the necessary directories in the RPM build root
mkdir -p %{buildroot}%{prefix}/bin
mkdir -p %{buildroot}%{prefix}/lib
mkdir -p %{buildroot}%{prefix}/lib64

# Copy only the desired directories to the RPM build root
cp -R %{_builddir}%{prefix}/bin/* %{buildroot}%{prefix}/bin/
cp -R %{_builddir}%{prefix}/lib/* %{buildroot}%{prefix}/lib/
cp -R %{_builddir}%{prefix}/lib64/* %{buildroot}%{prefix}/lib64/

# Remove the build directory
rm -rf %{_builddir}%{prefix}

echo "------------------------------------------------------------"
echo "Finished installing RsCap."
echo "------------------------------------------------------------"

# Create the RPM package
%files
%{prefix}/bin/*
%{prefix}/lib/*
%{prefix}/lib64/*

# Post installation script
%post
echo "%{prefix}/lib64" > %{_sysconfdir}/ld.so.conf.d/rscap64.conf
/sbin/ldconfig

# Post uninstallation script
%postun
if [ $1 -eq 0 ]; then
    rm -f %{_sysconfdir}/ld.so.conf.d/rscap64.conf
    /sbin/ldconfig
fi

# Clean up
%clean
rm -rf %{buildroot}
rm -rf %{_builddir}/*

%changelog
* Mon Apr 08 2024 Chris Kennedy <chris@rscap.com>
- Initial RPM release
