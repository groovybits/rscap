export PREFIX=/opt/rsprobe
export GST_PLUGIN_PATH=$PREFIX/lib64/gstreamer-1.0
export LD_LIBRARY_PATH=$PREFIX/lib64:$PREFIX/lib:$LD_LIBRARY_PATH
export PATH=$PREFIX/bin:$PATH

# For pkg-config to find .pc files
export PKG_CONFIG_PATH=$PREFIX/lib64/pkgconfig:$PREFIX/lib/pkgconfig:$PKG_CONFIG_PATH

