# MpegTS Stream Analysis Probe with Kafka and GStreamer

[![Rust](https://github.com/groovybits/rscap/actions/workflows/rust.yml/badge.svg?branch=main)](https://github.com/groovybits/rscap/actions/workflows/rust.yml)

## Overview

This project is an experiment researching Rust and its efficiency at handling high rates of streaming MpegTS for broadcast monitoring usage. The goal is to distribute a PCap-sourced MpegTS multicast network stream and send metrics with image assets and stream metadata to Kafka. The project captures the MpegTS using pcap with filter rules for specifying the stream IP and port, and validates the stream for conformance. If requested, metrics are sent to Kafka for long-term storage.

![RsProbe](https://storage.googleapis.com/groovybits/images/rscap/rscap.webp)

Gstreamer support is available with the `--features gst` flag (`make build_gst`) for using Gstreamer for stream demuxing/decoding. Currently, `--extract-images` will extract images from the stream and save them to disk or send them off to a Kafka feed as base64 with JSON metadata. See [scripts/probe.sh](scripts/probe.sh) for examples of how to use RsProbe in a common use case.

## The Probe Client

The [src/bin/probe.rs](src/bin/probe.rs) takes MpegTS via Packet Capture and publishes batches of the MpegTS 188 sized byte packets to a ZeroMQ socket it binds with PUSH. The probe has zero copy of the pcap buffers for the life cycle. It analyzes the MpegTS currently, a goal is to also support SMPTE2110 streams with DPDK, which is a work in progress and needs development and testing to completely optimize the behavior.

## Configuration with Environment Variables

Use `.env` and/or command line args to override the default/env variables. See [.env.example](.env.example) for an example configuration.

## RsProbe RPM for CentOS 7 with Gstreamer Support

[specs/rsprobe.spec](specs/rsprobe.spec) builds for CentOS 7 with all the Gstreamer build dependencies handled for you.

```
rpmbuild -bb specs/rsprobe.spec
```

## Building and Executing RsProbe with Gstreamer Dependencies

- RsProbe + Gstreamer Install script: [scripts/install.sh](scripts/install.sh) for Gstreamer + deps setup on Linux CentOS 7 and macOS into the `/opt/rsprobe` contained directory.
- RsProbe Compile script: [scripts/compile.sh](scripts/compile.sh) for RsProbe build using Gstreamer setup in `/opt/rsprobe`.

```text
# Install RsProbe w/gstreamer in /opt/rsprobe/ (MacOS or CentOS 7)
./scripts/install.sh

# Optionally rebuild RsProbe if making changes
./scripts/compile.sh gst

# Run the probe
./scripts/probe.sh
```

Output by default is the original data packet. You can add the JSON header with `--send-json-header`.

## Kafka Output of JSON Metrics

After processing and extraction, the project sends JSON metrics to Kafka. See the [Kafka Schema](kafka_schema/kafka.json) for the format of the data sent to Kafka.

```text
        --kafka-broker sun:9092 \
        --kafka-topic test \
        --send-to-kafka
```

## Profiling with Intel VTune (Linux/Windows)

Get VTune: [Intel oneAPI Base Toolkit](https://software.intel.com/content/www/us/en/develop/tools/oneapi/base-toolkit/download.html)

Running VTune [scripts/vtune.sh](scripts/vtune.sh)

```text
## Runtime for VTune
# Web UI (Best) Read [Intel VTune Documentation](https://www.intel.com/content/www/us/en/docs/vtune-profiler/user-guide/2024-0/web-server-ui.html)
./scripts/vtune.sh
```

## TODO - Roadmap Plans

- Audio analysis, capture, sampling showing amplitude graphs and noise violations of various broadcasting regulations.
- Caption packets and other NAL and SEI data / metadata extraction and sending.
- (WIP) Add more information header to the stream data metadata like network stats, mediainfo, captions, ancillary data.
- SMPTE 2110 handling reassembling frames and analogous to the MpegTS support.
- (WIP) PES parsing and analysis of streams.
- (WIP) FFmpeg libzmq protocol compatibility to allow branching off into libav easily.
- (WIP) General network analyzer view of network around the streams we know/care about.
- Use [OpenCV img_hash fingerprinting](https://docs.opencv.org/3.4/d4/d93/group__img__hash.html#ga5eeee1e27bc45caffe3b529ab42568e3) to perceptually align and compare video streams frames.
- OpenAI Whisper speech to text for caption verification and insertion. <https://github.com/openai/whisper>
- Problem discovery and reporting via LLM/VectorDB analysis detection of anomalies in data.
- Fine-tune LLM model for finding stream issues beyond basic commonly used ones.
- Segmentation of captured MpegTS, VOD file writer by various specs.
- Compression for proxy capture. Encode bitrate ladders realtime in parallel?
- SMPTE2110 data stream and audio stream support (need to have more than one pcap ip/port and distinguish them apart).
- Meme-like overlay of current frame and stream metrics on the thumbnail images with precise timing and frame information like a scope. (phone/pad usage)

![RsProbe](https://storage.googleapis.com/groovybits/images/rscap/rscap_circuit.webp)

### Chris Kennedy (C) 2024 MIT License
