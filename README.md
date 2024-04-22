# MpegTS Stream Analysis Probe in Rust

[![Rust](https://github.com/groovybits/rscap/actions/workflows/rust.yml/badge.svg?branch=main)](https://github.com/groovybits/rscap/actions/workflows/rust.yml)

## Overview

Capture MpegTS off a network and analyze for broadcast monitoring purposes. Measure various properties and extract assets from a PCap-sourced MpegTS multicast network stream. Efficiently send metrics with image assets and stream metadata to Kafka for display in graphs and charts with alerts setup on data points. The project captures the MpegTS using pcap with filter rules for specifying the stream IP and port, then validates the stream for conformance. If requested, metrics are sent to Kafka for long-term storage along with images and other stream metadata using Gstreamer to extract assets.

![RsProbe](https://storage.googleapis.com/groovybits/images/rscap/rscap.webp)

Gstreamer support is available with the `--features gst` flag (`make build_gst`) for using Gstreamer for stream demuxing/decoding. Currently, `--extract-images` will extract images from the stream and save them to disk or send them off to a Kafka feed as base64 with JSON metadata. See [scripts/probe.sh](scripts/probe.sh) for examples of how to use RsProbe in a common use case.

## The Probe Client

The [src/bin/probe.rs](src/bin/probe.rs) is the main entry point for the probe client. It captures the MpegTS stream and extracts metrics and assets from the stream. The probe client is the main entry point for the project and is the main executable for the project.

## Configuration with Environment Variables

Use `.env` and/or command line args to override the default/env variables. See [.env.example](.env.example) for an example configuration. The command line args are the same as the environment variables but with `--` prepended to the variable name and lower case.

## RsProbe RPM for CentOS 7 with Gstreamer Support

[specs/rsprobe.spec](specs/rsprobe.spec) builds for CentOS 7 with all the Gstreamer build dependencies handled for you.

```text
rpmbuild -bb specs/rsprobe.spec
```

## Docker Image for RsProbe

The Dockerfile [Dockerfile](Dockerfile) in the root of the project builds a CentOS 7 image with RsProbe installed. The image has various env variables matching the .env file for configuration. The image is built with the `--build-arg` flag for the environment variables.

## Building and Executing RsProbe with Gstreamer Dependencies

- RsProbe + Gstreamer Install script: [scripts/install.sh](scripts/install.sh) for Gstreamer + deps setup on Linux CentOS 7 and macOS into the `/opt/rsprobe` contained directory.
- RsProbe Compile script: [scripts/compile.sh](scripts/compile.sh) for RsProbe build using Gstreamer setup in `/opt/rsprobe`.

```text
# Install RsProbe w/gstreamer in /opt/rsprobe/ (MacOS or CentOS 7)
./scripts/install.sh # or make install

# Optionally rebuild RsProbe if making changes
./scripts/compile.sh gst # or make build_gst

# Run the probe
./scripts/probe.sh -h # help to see what options are available
```

## Kafka Output of JSON Metrics

After processing and extraction, the project sends JSON metrics to Kafka. See the [Kafka Schema](kafka_schema/kafka.json) for the format of the data sent to Kafka.

```text
        --kafka-broker sun:9092 \
        --kafka-topic test \
        --send-to-kafka
        --kafka-interval 1000 \ # in milliseconds
```

## TODO - Roadmap Plans

- Closed captioning extraction and analysis for compliance.
- Audio analysis and EBU R128 loudness monitoring.
- Audio levels and audio stream analysis.
- (WIP) SCTE-35 and other ad insertion markers extraction and analysis.
- SMPTE 2110 handling reassembling frames and analogous to the MpegTS support.
- SMPTE2110 data stream and audio stream support (need to have more than one pcap ip/port and distinguish them apart).
- (WIP) Direct raw PES parsing and analysis of streams.
- Network analyzer view of the other traffic on the network.
- Use [OpenCV img_hash fingerprinting](https://docs.opencv.org/3.4/d4/d93/group__img__hash.html#ga5eeee1e27bc45caffe3b529ab42568e3) to perceptually align and compare video streams frames.
- OpenAI Whisper speech to text for caption verification and insertion. <https://github.com/openai/whisper>
- Problem discovery and reporting via LLM/VectorDB analysis detection of anomalies in data. Use RsLLM for this. [RsLLM](https://github.com/groovybits/rsllm)
- Fine-tune LLM model for finding stream issues beyond basic commonly used ones.
- Segmentation of captured MpegTS, VOD file writer by various specs.
- Compression for proxy capture. Encode bitrate ladders realtime in parallel?
- Meme-like overlay of current frame and stream metrics on the thumbnail images with precise timing and frame information like a scope. (phone/pad usage)

![RsProbe](https://storage.googleapis.com/groovybits/images/rscap/rscap_circuit.webp)

## Development and Profiling with Intel VTune (Linux/Windows)

Get VTune: [Intel oneAPI Base Toolkit](https://software.intel.com/content/www/us/en/develop/tools/oneapi/base-toolkit/download.html)

Running VTune [scripts/vtune.sh](scripts/vtune.sh)

```text
## Runtime for VTune
# Web UI (Best) Read [Intel VTune Documentation](https://www.intel.com/content/www/us/en/docs/vtune-profiler/user-guide/2024-0/web-server-ui.html)
./scripts/vtune.sh
```

### Chris Kennedy (C) 2024 MIT License
