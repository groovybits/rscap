## Packet Capture MpegTS/SMPTE2110 for ZeroMQ Distributed Processing

Distribute an MpegTS network stream over ZeroMQ. Capture
the TS using pcap with filter rules for specifying
which stream. Validate the stream for conformance
keeping the ZeroMQ output clean without any non-legal
TS packets. Store metadata extracted in zeromq json headers.
Share out multicast to many clients for distributed stream processing.

![rscap](https://storage.googleapis.com/gaib/2/rscap/rscap.png)

## This consists of two programs, a probe and a client.

- The probe takes MpegTS via Packet Capture and publishes
batches of the MpegTS 188 byte packets to a ZeroMQ output.

- The client reads from the ZeroMQ socket and writes out a
a file containing the MpegTS stream matching the one
captured by the probe.

## Configuration with environment variables using [.env](.env.example)

```text
## rsCap Configuration
RUST_LOG="info" # debug, info, error

DEBUG=true
#SILENT=true
SEND_JSON_HEADER=true # Send metadata in a json header
USE_WIRELESS=true # Allow wireless interface usage

# ZeroMQ output host and port to TCP Publish
TARGET_IP="127.0.0.1"
TARGET_PORT=5556

# Pcap device to listen to, empty for autodetection
SOURCE_DEVICE=""

# Pcap filter for MpegTS multicast host and port
SOURCE_IP="224.0.0.200"
SOURCE_PORT=10000

# Output file name for client capture from ZeroMQ output
OUTPUT_FILE=capture.ts
```

## Building and executing

Install Rust via Homebrew on MacOS or from Rust main website (preferred)...

<https://www.rust-lang.org/tools/install>

```text
# CentOS 7.9
sudo yum group install "Development Tools"
sudo yum install centos-release-scl
sudo yum install devtoolset-11
scl enable devtoolset-11 bash
# End of CentOS 7.9

# MacOS Brew
brew install rust
# better...
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Build and run the pcap stream probe...

```text
cargo build
sudo target/debug/probe
```

Build and run the zmq capture client...

```text
target/debug/client
```

Check the output file capture.ts (or what you set in .env or environment variables)

```text
ffmpeg -i capture.ts
```

## TODO - roadmap plans

- (WIP) Add more information header to the json metadata like system stats, network stats, mediainfo, captions, ancillary data.
- (WIP) SMPTE 2110 handling analogous to the MpegTS support.
- (WIP) PAT/PMT parsing, PES parsing and analysis of streams.
- (WIP) FFmpeg libzmq protocol compatibility to allow branching off into libav easily.
- (WIP) Queue and Broker distribution robustness to allow large video streams capture without loss.
- (WIP) General network analyzer view of network around the streams we know/care about.
- Have multiple client modes to distribute processing of the stream on the zmq endpoints.
- Wrap [ltntstools](https://github.com/LTNGlobal-opensource/libltntstools) lib functionality into Rust through C bindings (If possible).
- SEI metadata decoding various aspects of MpegTS.
- Logging to file/sqliteDB with stats for simple basic graphing using gnuplot.
- Use [OpenCV img_hash fingerprinting](https://docs.opencv.org/3.4/d4/d93/group__img__hash.html#ga5eeee1e27bc45caffe3b529ab42568e3) to perceptually align and compare video streams frames.
- OpenAI Whisper speech to text for caption verfication and insertion. <https://github.com/openai/whisper>
- Problem discovery and reporting via LLM/VectorDB analysis detection of anomalies in data.
- Fine tune LLM model for finding stream issues beyond basic commonly used ones.
- Multiple streams?
- Segmentation of captured MpegTS, VOD file writer by various specs.
- Compression for proxy capture.

### Chris Kennedy (C) 2023 LGPL

