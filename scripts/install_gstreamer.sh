#!/bin/sh
set -e

# Function to run a command within the SCL environment for CentOS
run_with_scl() {
    OS="$(uname -s)"
    if [ "$OS" = "Linux" ]; then
        scl enable devtoolset-11 -- "$@"
    else
        "$@"
    fi
}

# Detect the operating system
OS="$(uname -s)"
echo "Detected OS: $OS"

BUILD_DIR=$(pwd)/build
if [ ! -d $BUILD_DIR ]; then
    mkdir -p $BUILD_DIR
fi
cd $BUILD_DIR

# Define versions for dependencies and GStreamer
ORC_VERSION=0.4.31
GST_VERSION=1.20.0
LIBFFI_VERSION=3.3
GST_PLUGINS_RS_VERSION=gstreamer-1.24.2
NASM_VERSION=2.15.05
FFMPEG_VERSION=5.1.4

# Define the installation prefix
PREFIX=/opt/rscap
export PATH=$PREFIX/bin:$PATH

USER=$(whoami)
sudo mkdir -p $PREFIX
sudo chown $USER $PREFIX

# For pkg-config to find .pc files
export PKG_CONFIG_PATH=$PREFIX/lib64/pkgconfig:$PREFIX/lib/pkgconfig:/usr/lib64/pkgconfig:$PKG_CONFIG_PATH
export LD_LIBRARY_PATH=$PREFIX/lib64:$PREFIX/lib:$LD_LIBRARY_PATH

export PATH=$PREFIX/bin:$PATH

if [ "$OS" = "Linux" ]; then
    # Ensure the system is up to date and has the basic build tools
    sudo yum groupinstall -y "Development Tools"
    sudo yum install -y bison flex python3 wget libffi-devel util-linux libmount-devel libxml2-devel glib2-devel zvbi-devel pango-devel cairo-devel capnproto-devel capnproto ladspa-devel
else
    export CXXFLAGS="-stdlib=libc++"
    export LDFLAGS="-lc++"
    export CXXFLAGS="-std=c++11"
fi

# Explicitly use cmake from $PREFIX/bin for Meson configuration
echo "[binaries]" > meson-native-file.ini
if [ "$OS" = "Linux" ]; then
    echo "cmake = 'cmake3'" >> meson-native-file.ini
else
    echo "cmake = 'cmake'" >> meson-native-file.ini
fi
CWD=$(pwd)
MESON_NATIVE_FILE=$CWD/meson-native-file.ini

# Ensure Meson and Ninja are installed and use the correct Ninja
if [ "$OS" = "Linux" ]; then
    sudo pip3 install meson
    sudo pip3 install ninja
else
    brew install meson
    brew install ninja
    brew install pkg-config
    brew install bison
    brew install wget
    export PATH="/usr/local/opt/bison/bin:$PATH"
fi

echo "------------------------------------------------------------"
echo "Installing GStreamer and essential dependencies..."
echo "------------------------------------------------------------"

# Install libFFI
if [ "$OS" = "Linux" ]; then
    if [ ! -f "libffi-installed.done" ] ; then
        echo "---"
        echo "Installing libffi..."
        echo "---"
        # Download, compile, and install libffi
        if [ ! -f libffi-$LIBFFI_VERSION.tar.gz ]; then
            curl ftp://sourceware.org/pub/libffi/libffi-$LIBFFI_VERSION.tar.gz -o libffi-$LIBFFI_VERSION.tar.gz
        fi
        if [ ! -d libffi-$LIBFFI_VERSION ]; then
            tar xf libffi-$LIBFFI_VERSION.tar.gz
        fi
        cd libffi-$LIBFFI_VERSION
        run_with_scl ./configure --prefix=$PREFIX
        run_with_scl make
        make install
        cd ..
    fi
    touch libffi-installed.done
else
    brew install libffi
fi

# Install ORC
if [ "$OS" = "Linux" ]; then
    if [ ! -f "orc-installed.done" ] ; then
        echo "---"
        echo "Installing ORC..."
        echo "---"
        # Download, compile, and install ORC
        if [ ! -f orc-$ORC_VERSION.tar.xz ]; then
            curl https://gstreamer.freedesktop.org/src/orc/orc-$ORC_VERSION.tar.xz -o orc-$ORC_VERSION.tar.xz
        fi
        if [ ! -d orc-$ORC_VERSION ]; then
            tar xf orc-$ORC_VERSION.tar.xz
        fi
        cd orc-$ORC_VERSION
        run_with_scl meson _build --prefix=$PREFIX --buildtype=release --native-file $MESON_NATIVE_FILE
        run_with_scl ninja -C _build
        ninja -C _build install
        cd ..
    fi
    touch orc-installed.done
else
    brew install orc
fi

# Download and compile NASM
if [ "$OS" = "Linux" ]; then
    if [ ! -f "nasm-installed.done" ] ; then
        if [ ! -f nasm-$NASM_VERSION.tar.gz ]; then
            curl https://www.nasm.us/pub/nasm/releasebuilds/$NASM_VERSION/nasm-$NASM_VERSION.tar.gz -o nasm-$NASM_VERSION.tar.gz
        fi
        # Extract
        if [ ! -d nasm-$NASM_VERSION ]; then
            tar -xzf nasm-$NASM_VERSION.tar.gz
        fi
        cd nasm-$NASM_VERSION
        # Compile and install
        ./autogen.sh
        ./configure --prefix=$PREFIX
        make
        make install
        cd ..
    fi
    touch nasm-installed.done
else
    brew install nasm
fi

# libx264
if [ "$OS" = "Linux" ]; then
    if [ ! -f "x264-installed.done" ] ; then
        echo "---"
        echo "Downloading and compiling libx264..."
        echo "---"
        echo "---"
        echo "Cloning and compiling libx264..."
        echo "---"
        # Ensure git is installed
        sudo yum install -y git

        # Clone the repository
        if [ ! -d "x264" ]; then
            git clone https://code.videolan.org/videolan/x264.git
        fi
        cd x264

        # Compile
        run_with_scl ./configure --prefix=$PREFIX --enable-shared --enable-static --enable-pic
        run_with_scl make
        make install
        sudo ldconfig
        cd ..
    fi
    touch x264-installed.done
else
    brew install x264
fi

# libx265
if [ "$OS" = "Linux" ]; then
    if [ ! -f "x265-installed.done" ] ; then
        echo "---"
        echo "Cloning and compiling x265..."
        echo "---"
        # Ensure necessary tools are installed
        sudo yum install -y cmake3 git

        # Clone the x265 repository if it doesn't already exist
        if [ ! -d "x265" ]; then
            git clone https://github.com/videolan/x265.git
        fi
        cd x265

        # Create a build directory and navigate into it
        mkdir -p build
        cd build

        # Use cmake3 to configure the build, respecting the PREFIX variable for installation
        run_with_scl cmake3 -G "Unix Makefiles" -DCMAKE_INSTALL_PREFIX=$PREFIX -DENABLE_SHARED:bool=on ../source

        # Compile and install
        run_with_scl make
        make install
        sudo ldconfig

        # Navigate back to the initial directory
        cd ../..
    fi
    touch x265-installed.done
else
    brew install x265
fi

# FFmpeg
#if [ "$OS" = "Linux" ]; then
    if [ ! -f "ffmpeg-installed.done" ] ; then
        echo "---"
        echo "Downloading and compiling FFmpeg..."
        echo "---"
        # Download
        if [ ! -f ffmpeg-$FFMPEG_VERSION.tar.bz2 ]; then
            wget http://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.bz2
        fi
        # Extract
        if [ ! -d ffmpeg-$FFMPEG_VERSION ]; then
            tar xf ffmpeg-$FFMPEG_VERSION.tar.bz2
        fi
        # Compile
        cd ffmpeg-$FFMPEG_VERSION
        run_with_scl ./configure --prefix=$PREFIX \
            --enable-shared --enable-static \
            --enable-pic --enable-gpl --enable-libx264 \
            --enable-libx265 \
            --extra-cflags="-I$PREFIX/include" --extra-ldflags="-L$PREFIX/lib"
        run_with_scl make
        make install
        if [ "$OS" = "Linux" ]; then
            sudo ldconfig
        fi
        cd ..
    fi
    touch ffmpeg-installed.done
#else
#    brew install ffmpeg
#fi

# Install Gstreamer core
if [ ! -f "gstreamer-installed.done" ] ; then
    echo "---"
    echo "Installing Gstreamer core..."
    echo "---"
    # Download, compile, and install GStreamer core
    if [ ! -f gstreamer-$GST_VERSION.tar.xz ]; then
        curl https://gstreamer.freedesktop.org/src/gstreamer/gstreamer-$GST_VERSION.tar.xz -o gstreamer-$GST_VERSION.tar.xz
    fi
    if [ ! -d gstreamer-$GST_VERSION ]; then
        tar xf gstreamer-$GST_VERSION.tar.xz
    fi
    cd gstreamer-$GST_VERSION
    run_with_scl meson _build --prefix=$PREFIX --buildtype=release --native-file $MESON_NATIVE_FILE
    run_with_scl ninja -C _build
    ninja -C _build install
    cd ..
fi
touch gstreamer-installed.done

# Install GStreamer base plugins
if [ ! -f "gst-plugins-base-installed.done" ] ; then
    echo "---"
    echo "Installing Gstreamer base..."
    echo "---"
    # Download, compile, and install gst-plugins-base
    if [ ! -f gst-plugins-base-$GST_VERSION.tar.xz ]; then
        curl https://gstreamer.freedesktop.org/src/gst-plugins-base/gst-plugins-base-$GST_VERSION.tar.xz -o gst-plugins-base-$GST_VERSION.tar.xz
    fi
    if [ ! -d gst-plugins-base-$GST_VERSION ]; then
        tar xf gst-plugins-base-$GST_VERSION.tar.xz
    fi
    cd gst-plugins-base-$GST_VERSION
    run_with_scl meson _build --prefix=$PREFIX --buildtype=release --native-file $MESON_NATIVE_FILE
    run_with_scl ninja -C _build
    ninja -C _build install
    cd ..
fi
touch gst-plugins-base-installed.done

# Install GStreamer bad plugins (includes tsdemux)
if [ ! -f "gst-plugins-bad-installed.done" ] ; then
    # Install GStreamer bad plugins (includes tsdemux)
    echo "---"
    echo "Installing Gstreamer bad plugins..."
    echo "---"
    if [ ! -f gst-plugins-bad-$GST_VERSION.tar.xz ]; then
        curl https://gstreamer.freedesktop.org/src/gst-plugins-bad/gst-plugins-bad-$GST_VERSION.tar.xz -o gst-plugins-bad-$GST_VERSION.tar.xz
    fi
    if [ ! -d gst-plugins-bad-$GST_VERSION ]; then
        tar xf gst-plugins-bad-$GST_VERSION.tar.xz
    fi
    cd gst-plugins-bad-$GST_VERSION
    run_with_scl meson _build --prefix=$PREFIX --buildtype=release --native-file $MESON_NATIVE_FILE -Dopenexr=disabled
    run_with_scl ninja -C _build
    ninja -C _build install
    cd ..
fi
touch gst-plugins-bad-installed.done

echo "---"
echo "Downloading and compiling NASM (Netwide Assembler)..."
echo "---"

# GStreamer libav plugins
if [ ! -f "gst-libav-installed.done" ] ; then
    echo "---"
    echo "Installing Gstreamer libav plugins..."
    echo "---"

    PWD=$(pwd)
    echo "PWD: $PWD"

    if [ ! -f gst-libav-$GST_VERSION.tar.xz ]; then
        curl https://gstreamer.freedesktop.org/src/gst-libav/gst-libav-$GST_VERSION.tar.xz -o gst-libav-$GST_VERSION.tar.xz
    fi
    if [ ! -d gst-libav-$GST_VERSION ]; then
        tar xf gst-libav-$GST_VERSION.tar.xz
    fi
    cd gst-libav-$GST_VERSION
    run_with_scl meson _build --prefix=$PREFIX --buildtype=release --native-file $MESON_NATIVE_FILE --pkg-config-path=$PKG_CONFIG_PATH
    run_with_scl ninja -C _build
    ninja -C _build install
    cd ..
fi
touch gst-libav-installed.done

# GStreamer good plugins
if [ ! -f "gst-plugins-good-installed.done" ] ; then
    echo "---"
    echo "Installing GStreamer good plugins..."
    echo "---"
    # Download, compile, and install gst-plugins-good
    if [ ! -f gst-plugins-good-$GST_VERSION.tar.xz ]; then
        curl https://gstreamer.freedesktop.org/src/gst-plugins-good/gst-plugins-good-$GST_VERSION.tar.xz -o gst-plugins-good-$GST_VERSION.tar.xz
    fi
    if [ ! -d gst-plugins-good-$GST_VERSION ]; then
        tar xf gst-plugins-good-$GST_VERSION.tar.xz
    fi
    cd gst-plugins-good-$GST_VERSION
    run_with_scl meson _build --prefix=$PREFIX --buildtype=release --native-file $MESON_NATIVE_FILE
    run_with_scl ninja -C _build
    ninja -C _build install
    cd ..
fi
touch gst-plugins-good-installed.done

if [ ! -f "gst-plugins-rs-installed.done" ]; then
  echo "---"
  echo "Installing GStreamer Rust plugins..."
  echo "---"

  # Install cargo-c
  run_with_scl cargo install cargo-c --root $PREFIX

  # Download gst-plugins-rs source code
  if [ ! -f gst-plugin-rs ]; then
    git clone https://github.com/sdroege/gst-plugin-rs.git
    cd gst-plugin-rs
    git checkout $GST_PLUGINS_RS_VERSION
    cd ..
  fi

  USER=$(whoami)
  sudo chown -R $USER /opt/rscap

  # Build gst-plugin-cdg
  cd gst-plugin-rs
  run_with_scl cargo cbuild --release --package gst-plugin-cdg
  run_with_scl cargo cinstall --release --package gst-plugin-cdg --prefix=$PREFIX --libdir=$PREFIX/lib64
  run_with_scl cargo cbuild --release --package gst-plugin-cdg
  run_with_scl cargo cbuild --release --package gst-plugin-closedcaption
  run_with_scl cargo cinstall --release --package gst-plugin-closedcaption --prefix=$PREFIX --libdir=$PREFIX/lib64
  cd ..
  rm -rf gst-plugin-rs
fi

touch gst-plugins-rs-installed.done

# Verify GStreamer installation
echo "------------------------------------------------------------"
echo "Verifying GStreamer installation..."
echo "------------------------------------------------------------"
gst-launch-1.0 --version

echo "------------------------------------------------------------"
echo "GStreamer and essential dependencies installed."
echo "------------------------------------------------------------"
