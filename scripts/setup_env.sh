PREFIX=/opt/rscap
GST_PLUGIN_PATH=$PREFIX/lib64/gstreamer-1.0
LD_LIBRARY_PATH=$PREFIX/lib64:$PREFIX/lib:$LD_LIBRARY_PATH
PATH=$PREFIX/bin:$PATH

# For pkg-config to find .pc files
export PKG_CONFIG_PATH=$PREFIX/lib64/pkgconfig:$PREFIX/lib/pkgconfig:/usr/lib64/pkgconfig:$PKG_CONFIG_PATH
