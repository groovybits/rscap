FROM centos:7 as builder

# Set up working directory
WORKDIR /app

COPY . /app/
WORKDIR /app

# Install required packages
RUN yum groupinstall -y "Development Tools" && \
    yum install -y centos-release-scl-rh epel-release && \
    yum install -y devtoolset-11 rh-python38 && \
    yum install -y yum-utils && \
    yum install --enablerepo=epel* -y zvbi-devel && \
    yum install -y --enablerepo=epel* wget curl git cmake3 && \
    yum install -y cmake3 git libstdc++-devel gcc gcc-c++ make && \
    yum install -y bison flex python3 wget libffi-devel \
    util-linux libmount-devel libxml2-devel glib2-devel \
    cairo-devel ladspa-devel pango-devel cairo-gobject-devel cairo-gobject && \
    yum install -y llvm-toolset-7.0-llvm-devel llvm-toolset-7.0-clang && \
    yum install -y rh-python38 rh-python38-python-pip && \
    yum install -y gcc gcc-c++ \
    make python3 wget libffi-devel util-linux libmount-devel bison flex git \
    libxml2-devel pango-devel cairo-devel zvbi-devel ladspa-devel cairo-gobject-devel \
    cairo-gobject rh-python38 rh-python38-python-pip llvm-toolset-7.0-clang-devel libstdc++-devel \
    llvm llvm-devel libjpeg-turbo-devel libtiff-devel llvm-toolset-7.0-llvm-devel llvm-toolset-7.0-clang

## Install the newest Rust Compiler
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    RUSTUP_INIT_SKIP_PATH_CHECK=yes \
    CARGO_HOME=/usr RUSTUP_HOME=/usr sh \
    -s -- -y --no-modify-path --default-toolchain stable && rustup default stable

## Install Meson/Ninja and Rscap
RUN pip3 install meson && scl enable devtoolset-11 rh-python38 -- pip3.8 install meson
RUN pip3 install ninja && scl enable devtoolset-11 rh-python38 -- pip3.8 install ninja
RUN sh ./scripts/install.sh

FROM centos:7 AS binary

# Copy the installed RsProbe files from the builder stage
COPY --from=builder /opt/rsprobe /opt/rsprobe

# Install required runtime dependencies
RUN yum install -y libpcap zlib glibc-devel libstdc++ && \
    yum clean all

# Set environment variables
ARG SOURCE_DEVICE=eth0
ARG SOURCE_IP=224.0.0.200
ARG SOURCE_PORT=10001
ARG KAFKA_KEY=
ARG KAFKA_TOPIC=rsprobe
ARG KAFKA_BROKER=localhost:9092
ARG EXTRACT_IMAGES=true

ENV RUST_LOG="error"
ENV SOURCE_DEVICE=${SOURCE_DEVICE}
ENV SOURCE_IP=${SOURCE_IP}
ENV SOURCE_PORT=${SOURCE_PORT}
ENV KAFKA_KEY=${KAFKA_KEY}
ENV KAFKA_TOPIC=${KAFKA_TOPIC}
ENV KAFKA_BROKER=${KAFKA_BROKER}
ENV EXTRACT_IMAGES=${EXTRACT_IMAGES}

# Set the entrypoint
ENTRYPOINT ["/opt/rsprobe/bin/probe", "--pcap-stats"]
