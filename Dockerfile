FROM centos:7 as builder

# Install required packages
RUN yum install -y epel-release centos-release-scl && \
    yum install -y devtoolset-11 rh-python38 && \
    yum install -y wget curl git

# Set up working directory
WORKDIR /app

COPY . /app/
WORKDIR /app

RUN echo "alias sudo='true'" >> ~/.bashrc && source ~/.bashrc && sh ./scripts/install.sh

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
