# MpegTS/SMPTE2110 Stream Capture Monitoring in Rust

[![Rust](https://github.com/groovybits/rscap/actions/workflows/rust.yml/badge.svg?branch=main)](https://github.com/groovybits/rscap/actions/workflows/rust.yml)

An experiment researching Rust and efficiency at handling high rates of streaming MpegTS and SMPTE2110 for Broadcast Monitoring usage.
Distribute an PCap sourced MpegTS/SMPTE2110 multcast network stream and distribute to ZeroMQ Monitor module for sending to Kafka.
Capture the TS/SMPTE2110 using pcap with filter rules for specifying which stream ip and port. Validate the stream for conformance
keeping the ZeroMQ output clean without any non-legal TS/SMPTE2110 packets. Send metadata from probe to monitor via ZMQP serialized as Cap'n Proto buffers.
Send metrics to Kafka from the monitor process if requested for long-term storage.

Optionally the monitor process can output final json metrics to kafka for distributed probes sending to some kafka based centralized processing system for the data collected.

Uses H264_Reader <https://github.com/dholroyd/h264-reader> for NAL parsing.

TODO:
- Add AAC Parsing support <https://github.com/dholroyd/adts-reader> for Audio analysis.
- Add SCTE35 Trigger support <https://github.com/m2amedia/scte35dump> for Ad Cues.
- Integrate usage of MpegTS-Reader support <https://github.com/dholroyd/mpeg2ts-reader> for more advanced demuxing.
- Add System metrics <https://github.com/dholroyd/nix/tree/master> for view of OS health and load.
- Add Network metrics via pcap potentially or Rust crate, have not located one yet.
- Add HLS input support <https://github.com/dholroyd/hls_m3u8> for streams.
- Add decoder for viewing thumbnail of stream, potentially using <https://github.com/rust-av/rust-av> Rust AV pure rust, else binding libav C.

![rscap](https://storage.googleapis.com/gaib/2/rscap/rscap.png)

## This consists of two programs, a probe and a monitor client.

- The [src/bin/probe.rs](src/bin/probe.rs) takes MpegTS or SMPTE2110 via Packet Capture and publishes
batches of the MpegTS 188 / SMPTE2110 sized byte packets to a ZeroMQ output.

- The [src/bin/probe.rs](src/bin/probe.rs) has zero copy of the pcap buffers for the life cycle. They are passed through to the monitor module with Cap'n Proto allowing highly efficient capture and processing of both MpegTS and SMPTE2110 streams (WIP: needs work and testing to completely optimize the behavior).

- The [src/bin/monitor.rs](src/bin/monitor.rs) client reads from the ZeroMQ socket and writes out a
a file containing the MpegTS stream matching the one
captured by the probe.

## Configuration with environment variables using [.env](.env.example)

Use .env and/or command line args to override the default/env variables.

## Building and executing (see [compile.sh](compile.sh) for extensible setup Linux/MacOS)

Install Rust via Homebrew on MacOS or from Rust main website (preferred from main website)...

<https://www.rust-lang.org/tools/install>

```text
## How to install Rust ##

# --- CentOS 7.9
sudo yum group install "Development Tools"
sudo yum install centos-release-scl
sudo yum install devtoolset-11
scl enable devtoolset-11 bash
# --- End of CentOS 7.9

# --- MacOS Brew
brew install rust

# --- BEST OPTION...
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
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

Build and run the pcap stream probe [./compile.sh](compile.sh) (script takes care of most issues)

```text
# Compile release,release-with-debug, and release  versions in target/ directories.
./compile.sh

# Use ENV Variable RUST_LOG= for logging level and cmdline args
sudo RUST_LOG=info target/release/probe \
         --source-ip 224.0.0.1 \
         --source-port 10000 \
         --target-ip 127.0.0.1 \
         --target-port 5556 \
         --source-device eth0 \
         --debug

```

Output by default is the original data packet, you can add the json header with --send-json-header.

Build and run the zmq capture monitor client...

```text
# Use ENV Variable RUST_LOG= for logging level and cmdline args
RUST_LOG=info target/release/monitor \
         --source-ip 127.0.0.1 \
         --source-port 5556 \
         --output_file capture.ts \
         --debug

```

Check the output file capture.ts (or what you set in .env or environment variables)

```text
ffmpeg -i capture.ts
```

## Kafka output of json metrics from the monitor process after processing and extraction

- [Kafka Schema](schema/kafka.json) That is sent into Kafka
- [Kafka / Prometheus / Grafana / JS Dashboard UI](probe_ui) Allowing remove viewing of dashboards in Grafana/Prometheus/JS (WIP: not conforming schemawise yet)

```text
target/release/monitor \
        --kafka-broker sun:9092 \
        --kafka-topic test \
        --send-to-kafka
```

## Probe Command Line Options (as of 01/04/2024)

```text
RsCap Probe for ZeroMQ output of MPEG-TS and SMPTE 2110 streams from pcap.

Usage: probe [OPTIONS]

Options:
      --batch-size <BATCH_SIZE>
          Sets the batch size [env: BATCH_SIZE=] [default: 7]
      --payload-offset <PAYLOAD_OFFSET>
          Sets the payload offset [env: PAYLOAD_OFFSET=] [default: 42]
      --packet-size <PACKET_SIZE>
          Sets the packet size [env: PACKET_SIZE=] [default: 188]
      --read-time-out <READ_TIME_OUT>
          Sets the read timeout [env: READ_TIME_OUT=] [default: 60000]
      --target-port <TARGET_PORT>
          Sets the target port [env: TARGET_PORT=] [default: 5556]
      --target-ip <TARGET_IP>
          Sets the target IP [env: TARGET_IP=] [default: 127.0.0.1]
      --source-device <SOURCE_DEVICE>
          Sets the source device [env: SOURCE_DEVICE=] [default: ]
      --source-ip <SOURCE_IP>
          Sets the source IP [env: SOURCE_IP=] [default: 224.0.0.200]
      --source-protocol <SOURCE_PROTOCOL>
          Sets the source protocol [env: SOURCE_PROTOCOL=] [default: udp]
      --source-port <SOURCE_PORT>
          Sets the source port [env: SOURCE_PORT=] [default: 10000]
      --debug-on
          Sets the debug mode [env: DEBUG=]
      --silent
          Sets the silent mode [env: SILENT=]
      --use-wireless
          Sets if wireless is used [env: USE_WIRELESS=]
      --send-raw-stream
          Sets if Raw Stream should be sent [env: SEND_RAW_STREAM=]
      --packet-count <PACKET_COUNT>
          number of packets to capture [env: PACKET_COUNT=] [default: 0]
      --no-progress
          Turn off progress output dots [env: NO_PROGRESS=]
      --no-zmq
          Turn off ZeroMQ send [env: NO_ZMQ=]
      --smpte2110
          Force smpte2110 mode [env: SMPT2110=]
      --promiscuous
          Use promiscuous mode [env: PROMISCUOUS=]
      --show-tr101290
          Show the TR101290 p1, p2 and p3 errors if any [env: SHOW_TR101290=]
      --buffer-size <BUFFER_SIZE>
          Sets the pcap buffer size [env: BUFFER_SIZE=] [default: 1358000]
      --immediate-mode
          PCAP immediate mode [env: IMMEDIATE_MODE=]
      --pcap-stats
          PCAP output capture stats mode [env: PCAP_STATS=]
      --pcap-channel-size <PCAP_CHANNEL_SIZE>
          MPSC Channel Size for ZeroMQ [env: PCAP_CHANNEL_SIZE=] [default: 1000]
      --zmq-channel-size <ZMQ_CHANNEL_SIZE>
          MPSC Channel Size for PCAP [env: ZMQ_CHANNEL_SIZE=] [default: 1000]
      --decoder-channel-size <DECODER_CHANNEL_SIZE>
          MPSC Channel Size for Decoder [env: DECODER_CHANNEL_SIZE=] [default: 1000]
      --dpdk
          DPDK enable [env: DPDK=]
      --dpdk-port-id <DPDK_PORT_ID>
          DPDK Port ID [env: DPDK_PORT_ID=] [default: 0]
      --ipc-path <IPC_PATH>
          IPC Path for ZeroMQ [env: IPC_PATH=]
      --output-file <OUTPUT_FILE>
          Output file for ZeroMQ [env: OUTPUT_FILE=] [default: ]
      --no-zmq-thread
          Turn off the ZMQ thread [env: NO_ZMQ_THREAD=]
      --zmq-batch-size <ZMQ_BATCH_SIZE>
          ZMQ Batch size [env: ZMQ_BATCH_SIZE=] [default: 7]
      --decode-video
          Decode Video [env: DECODE_VIDEO=]
      --decode-video-batch-size <DECODE_VIDEO_BATCH_SIZE>
          Decode Video Batch Size [env: DECODE_VIDEO_BATCH_SIZE=] [default: 7]
      --debug-smpte2110
          Debug SMPTE2110 [env: DEBUG_SMPTE2110=]
      --debug-nals
          Debug NALs [env: DEBUG_NALS=]
      --debug-nal-types <DEBUG_NAL_TYPES>
          List of NAL types to debug, comma separated: sps, pps, pic_timing, sei, slice, unknown [env: DEBUG_NAL_TYPES=] [default: ]
      --parse-short-nals
          [env: PARSE_SHORT_NALS=]
  -h, --help
          Print help
  -V, --version
          Print version
```

## Monitor Command Line Options (as of 01/04/2024)

```text
RsCap Monitor for ZeroMQ input of MPEG-TS and SMPTE 2110 streams from remote probe.

Usage: monitor [OPTIONS]

Options:
      --source-port <SOURCE_PORT>      Sets the target port [env: TARGET_PORT=] [default: 5556]
      --source-ip <SOURCE_IP>          Sets the target IP [env: TARGET_IP=] [default: 127.0.0.1]
      --debug-on                       Sets the debug mode [env: DEBUG=]
      --silent                         Sets the silent mode [env: SILENT=]
      --recv-raw-stream                Sets if Raw Stream should be sent [env: RECV_RAW_STREAM=]
      --packet-count <PACKET_COUNT>    number of packets to capture [env: PACKET_COUNT=] [default: 0]
      --no-progress                    Turn off progress output dots [env: NO_PROGRESS=]
      --output-file <OUTPUT_FILE>      Output Filename [env: OUTPUT_FILE=] [default: ]
      --kafka-broker <KAFKA_BROKER>    Kafka Broker [env: KAFKA_BROKER=] [default: localhost:9092]
      --kafka-topic <KAFKA_TOPIC>      Kafka Topic [env: KAFKA_TOPIC=] [default: rscap]
      --send-to-kafka                  Send to Kafka if true [env: SEND_TO_KAFKA=]
      --kafka-timeout <KAFKA_TIMEOUT>  Kafka timeout to drop packets [env: KAFKA_TIMEOUT=] [default: 0]
      --ipc-path <IPC_PATH>            IPC Path for ZeroMQ [env: IPC_PATH=]
      --show-os-stats                  Show OS [env: SHOW_OS_STATS=]
  -h, --help                           Print help
  -V, --version                        Print version
```

## Profiling with Intel Vtune (Linux/Windows)

Get VTune: [Intel oneAPI Base Toolkit](https://software.intel.com/content/www/us/en/develop/tools/oneapi/base-toolkit/download.html)

Running VTune [vtune.sh](vtune.sh)

```text
## Runtime for VTune
# Web UI (Best) Read [Intel VTune Documentation](https://www.intel.com/content/www/us/en/docs/vtune-profiler/user-guide/2024-0/web-server-ui.html)
./vtune.sh
```

## TODO - roadmap plans

- (WIP) Add more information header to the stream data metadata like system stats, network stats, mediainfo, captions, ancillary data.
- (WIP) SMPTE 2110 handling reassembling frames and analogous to the MpegTS support.
- (WIP) PES parsing and analysis of streams.
- (WIP) FFmpeg libzmq protocol compatibility to allow branching off into libav easily.
- (WIP) General network analyzer view of network around the streams we know/care about.
- Have multiple client modes to distribute processing of the stream on the zmq endpoints.
- Wrap [ltntstools](https://github.com/LTNGlobal-opensource/libltntstools) lib functionality into Rust through C bindings (If possible).
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
