#!/bin/bash
#
set -e

git clone https://github.com/capnproto/capnproto.git

cd capnproto/c++
cmake .
make -j8 check  # Adjust '6' to match the number of CPU cores on your machine
sudo make install

