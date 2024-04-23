#!/bin/bash

set -e
#set -v

OS="$(uname -s)"

CPUS=
if [ "$OS" == "Linux" ]; then
    CPUS=$(nproc)
else
    CPUS=$(sysctl -n hw.ncpu)
fi

O0PENCV_VERSION=4.5.5
CMAKE=cmake
PREFIX=/opt/rsprobe

if [ "$OS" == "Linux" ]; then
    export CMAKE=cmake3
else
    brew install cmake
    export CMAKE=cmake
fi

run_with_scl() {
    OS="$(uname -s)"
    if [ "$OS" == "Linux" ]; then
        scl enable devtoolset-11 llvm-toolset-7.0 -- "$@"
    else
        "$@"
    fi
}

# CMD LINE ARGS
if [ "$1" == "" ]; then
    if [ ! -d "build" ]; then
        mkdir -p build
    fi

    cd build
    echo "Building OpenCV"
else
    echo "Building OpenCV"
    PREFIX=$1
fi

## Install OpenCV with perceptual image hashing
if [ ! -d "opencv" ]; then
    git clone https://github.com/opencv/opencv.git
    git checkout $OPENCV_VERSION
fi
if [ ! -d "opencv_contrib" ]; then
    git clone https://github.com/opencv/opencv_contrib.git
    git checkout $OPENCV_VERSION
fi

if [ -d "opencv/build" ]; then
    rm -rf opencv/build # fresh build
    mkdir opencv/build
else
    mkdir opencv/build
fi

export GST_PLUGIN_PATH=$PREFIX/lib64/gstreamer-1.0
export LD_LIBRARY_PATH=$PREFIX/lib64:$PREFIX/lib:$LD_LIBRARY_PATH
export PATH=$PREFIX/bin:$PATH

# For pkg-config to find .pc files
export PKG_CONFIG_PATH=$PREFIX/lib64/pkgconfig:$PREFIX/lib/pkgconfig:$PKG_CONFIG_PATH

cd opencv/build

CMAKE_C_COMPILER_VAR=
CMAKE_CXX_COMPILER_VAR=
if [ "$OS" == "Linux" ]; then
     CMAKE_C_COMPILER_VAR=-DCMAKE_C_COMPILER=/opt/rh/llvm-toolset-7.0/root/usr/bin/clang
     CMAKE_CXX_COMPILER_VAR=-DCMAKE_CXX_COMPILER=/opt/rh/llvm-toolset-7.0/root/usr/bin/clang++
fi

run_with_scl $CMAKE \
    -D CMAKE_BUILD_TYPE=RELEASE \
    -D CMAKE_INSTALL_PREFIX=$PREFIX \
    -D INSTALL_C_EXAMPLES=OFF \
    -D INSTALL_PYTHON_EXAMPLES=OFF \
    -DBUILD_opencv_core=ON \
    -DBUILD_opencv_imgproc=ON \
    -DBUILD_opencv_img_hash=ON \
    -DBUILD_opencv_imgcodecs=ON \
    -DBUILD_opencv_highgui=ON $CMAKE_C_COMPILER_VAR $CMAKE_CXX_COMPILER_VAR \
    -DWITH_TBB=ON \
    -DWITH_V4L=OFF \
    -DWITH_QT=OFF \
    -DWITH_OPENGL=ON \
    -DOPENCV_EXTRA_MODULES_PATH=../../opencv_contrib/modules \
    -DWITH_GSTREAMER=OFF \
    -DWITH_FFMPEG=OFF \
    -DOPENCV_GENERATE_PKGCONFIG=ON \
    -DBUILD_EXAMPLES=OFF .. --log-level=ERROR

echo "Configured OpenCV"
run_with_scl make -j $CPUS --silent
echo "Built OpenCV"
make install --silent
echo "OpenCV installed to $PREFIX"
