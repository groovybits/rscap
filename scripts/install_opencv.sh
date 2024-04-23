#!/bin/bash

set -e
#set -v

OS="$(uname -s)"

O0PENCV_VERSION=4.5.5
CMAKE=cmake
PREFIX=/opt/rsprobe

run_with_scl() {
    if [ "$OS" = "Linux" ]; then
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

if [ "$OS" == "Linux" ]; then
    export CMAKE=cmake3
else
    brew install cmake
    export CMAKE=cmake
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
     CMAKE_C_COMPILER_VAR="-D CMAKE_C_COMPILER=/opt/rh/llvm-toolset-7.0/root/usr/bin/clang"
     CMAKE_CXX_COMPILER_VAR="-D CMAKE_CXX_COMPILER=/opt/rh/llvm-toolset-7.0/root/usr/bin/clang++"
fi

run_with_scl $CMAKE -D CMAKE_BUILD_TYPE=RELEASE \
    -D CMAKE_INSTALL_PREFIX=$PREFIX \
    -D INSTALL_C_EXAMPLES=OFF \
    -D INSTALL_PYTHON_EXAMPLES=OFF \
    -DBUILD_opencv_core=ON \
    -DBUILD_opencv_imgproc=ON \
    -DBUILD_opencv_img_hash=ON \
    -DBUILD_opencv_imgcodecs=ON \
    -DBUILD_opencv_highgui=ON \
    $CMAKE_C_COMPILER_VAR \
    $CMAKE_CXX_COMPILER_VAR \
    -D WITH_TBB=ON \
    -D WITH_V4L=OFF \
    -D WITH_QT=OFF \
    -D WITH_OPENGL=ON \
    -D OPENCV_EXTRA_MODULES_PATH=../../opencv_contrib/modules \
    -D WITH_GSTREAMER=OFF \
    -D WITH_FFMPEG=OFF \
    -D OPENCV_GENERATE_PKGCONFIG=ON \
    -D BUILD_EXAMPLES=OFF ..

echo "Built OpenCV"
run_with_scl make
run_with_scl make install

echo "OpenCV installed to $PREFIX"
