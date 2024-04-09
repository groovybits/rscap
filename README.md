# MpegTS/SMPTE2110 Stream Capture Monitoring in Rust

[![Rust](https://github.com/groovybits/rscap/actions/workflows/rust.yml/badge.svg?branch=main)](https://github.com/groovybits/rscap/actions/workflows/rust.yml)

An experiment researching Rust and efficiency at handling high rates of streaming MpegTS and SMPTE2110 for Broadcast Monitoring usage.
Distribute an PCap sourced MpegTS/SMPTE2110 multcast network stream and distribute to ZeroMQ Monitor module for sending to Kafka.
Capture the TS/SMPTE2110 using pcap with filter rules for specifying which stream ip and port. Validate the stream for conformance
keeping the ZeroMQ output clean without any non-legal TS/SMPTE2110 packets. Send metadata from probe to monitor via ZMQP serialized as Cap'n Proto buffers.
Send metrics to Kafka from the monitor process if requested for long-term storage.

Optionally the monitor process can output final json metrics to kafka for distributed probes sending to some kafka based centralized processing system for the data collected.

Gstreamer support with --features gst `make build_gst` for using Gstreamer for stream demuxing/decoding. Currently `--extract-images` will extract images from the stream and save them to disk or send them off to a kafka feed as base64 with json metadata. See the scripts/monitor.sh and scripts/probe.sh for examples of how to use RsCap in a common use case.

Can enable MPEG2TS_Reader and H264_Reader crates from <https://github.com/dholroyd> for NAL parsing. The information from the SPS/PPS and other NAL data isn't currently used but will be added to the monitor process eventually to allow for a more detailed analysis of the stream.

TODO:

- Add AAC Parsing support <https://github.com/dholroyd/adts-reader> for Audio analysis.
- Add SCTE35 Trigger support <https://github.com/m2amedia/scte35dump> for Ad Cues.
- Integrate usage of MpegTS-Reader support <https://github.com/dholroyd/mpeg2ts-reader> for more advanced demuxing.
- Add System metrics <https://github.com/dholroyd/nix/tree/master> for view of OS health and load.
- Add Network metrics via pcap potentially or Rust crate, have not located one yet.
- Add HLS input support <https://github.com/dholroyd/hls_m3u8> for streams.

## This consists of two programs, a probe and a monitor client.

- The [src/bin/probe.rs](src/bin/probe.rs) takes MpegTS or SMPTE2110 via Packet Capture and publishes
  batches of the MpegTS 188 / SMPTE2110 sized byte packets to a ZeroMQ output.

- The [src/bin/probe.rs](src/bin/probe.rs) has zero copy of the pcap buffers for the life cycle. They are passed through to the monitor module with Cap'n Proto allowing highly efficient capture and processing of both MpegTS and SMPTE2110 streams (WIP: needs work and testing to completely optimize the behavior).

- The [src/bin/monitor.rs](src/bin/monitor.rs) client reads from the ZeroMQ socket and writes out a
  a file containing the MpegTS stream matching the one
  captured by the probe.

## Configuration with environment variables using [.env](.env.example)

Use .env and/or command line args to override the default/env variables.

## Building and executing (see [scripts/compile.sh](scripts/compile.sh) for extensible setup Linux/MacOS)

Install Rust via Homebrew on MacOS or from Rust main website (preferred from main website). Also the compile.sh script will install the necessary dependencies for CentOS 7.9 and MacOS.

```text
scripts/compile.sh gst # for Gstreamer support
```

On Linux update libpcap to the newest version (optional)...

```text
## Build and install newest pcap, check for newest release

# Build tools
sudo yum -y install flex                                                                                                                                       |
sudo yum -y install bison byacc yacc

# Pcap source
wget https://www.tcpdump.org/release/libpcap-1.10.4.tar.gz
tar xfz libpcap-1.10.4.tar.gz

# Pcap build procedure (Linux)
cd libpcap-1.10.4
./configure --prefix=/usr --libdir=/usr/lib64 --includedir=/usr/include --exec_prefix=/usr
make
sudo make install
```

Build and run the pcap stream probe [./scripts/compile.sh](scripts/compile.sh) (script takes care of most issues)

```text
# Compile the probe and monitor
./scripts/compile.sh

# Run the probe
./scripts/probe.sh
```

Output by default is the original data packet, you can add the json header with --send-json-header.

Build and run the zmq capture monitor client...

```text
# Run the monitor
./scripts/monitor.sh
```

## Kafka output of json metrics from the monitor process after processing and extraction

- [Kafka Schema](test_data/kafka.json) That is sent into Kafka

```text
scripts/monitor:

        --kafka-broker sun:9092 \
        --kafka-topic test \
        --send-to-kafka
```

## Profiling with Intel Vtune (Linux/Windows)

Get VTune: [Intel oneAPI Base Toolkit](https://software.intel.com/content/www/us/en/develop/tools/oneapi/base-toolkit/download.html)

Running VTune [scripts/vtune.sh](scripts/vtune.sh)

```text
## Runtime for VTune
# Web UI (Best) Read [Intel VTune Documentation](https://www.intel.com/content/www/us/en/docs/vtune-profiler/user-guide/2024-0/web-server-ui.html)
./scripts/vtune.sh
```

## TODO - roadmap plans

- (WIP) Add more information header to the stream data metadata like system stats, network stats, mediainfo, captions, ancillary data.
- (WIP) SMPTE 2110 handling reassembling frames and analogous to the MpegTS support.
- (WIP) PES parsing and analysis of streams.
- (WIP) FFmpeg libzmq protocol compatibility to allow branching off into libav easily.
- (WIP) General network analyzer view of network around the streams we know/care about.
- Have multiple client modes to distribute processing of the stream on the zmq endpoints.
- Improve NAL parsing and various aspects of MpegTS and VANC ancillary data from SMPTE2110.
- Logging to file/sqliteDB with stats for simple basic graphing using gnuplot.
- Use [OpenCV img_hash fingerprinting](https://docs.opencv.org/3.4/d4/d93/group__img__hash.html#ga5eeee1e27bc45caffe3b529ab42568e3) to perceptually align and compare video streams frames.
- OpenAI Whisper speech to text for caption verfication and insertion. <https://github.com/openai/whisper>
- Problem discovery and reporting via LLM/VectorDB analysis detection of anomalies in data.
- Fine tune LLM model for finding stream issues beyond basic commonly used ones.
- Segmentation of captured MpegTS, VOD file writer by various specs.
- Compression for proxy capture. Encode bitrate ladders realtime in parallel?
- Multiple streams per probe? Seems better to separate each probe to avoid a mess. TBD later, doubtful I like the idea.
- Audio analysis, capture, sampling showing amplitude graphs and noise violations of various broadcasting regulations.
- Thumbnail image extraction and compression for sending a thumbnail per second intervals (or less/more) to monitor in the monitor.
- Caption packets and other NAL and SEI data / metadata extraction and sending.
- SMPTE2110 data stream and audio stream support (need to have more than one pcap ip/port and distinguish them apart).
- Meme like overlay of current frame and stream metrics on the thumbnail images with precise timing and frame information like a scope. (phone/pad usage)

### Chris Kennedy (C) 2024 MIT License
