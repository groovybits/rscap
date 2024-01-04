# MpegTS/SMPTE2110 Stream Capture Monitoring in Rust

[![Rust](https://github.com/groovybits/rscap/actions/workflows/rust.yml/badge.svg?branch=main)](https://github.com/groovybits/rscap/actions/workflows/rust.yml)

An experiment researching Rust and efficiency at handling high rates of streaming MpegTS and SMPTE2110 for Broadcast Monitoring usage.
Distribute an PCap sourced MpegTS/SMPTE2110 multcasted network stream and distribute to ZeroMQ Monitor modules.
Capture the TS/SMPTE2110 using pcap with filter rules for specifying which stream ip and port. Validate the stream for conformance
keeping the ZeroMQ output clean without any non-legal TS/SMPTE2110 packets. Store metadata extracted in zeromq json headers (Cap'n Proto soon).
Share out multicast to many clients for distributed stream processing. Zero pcap buffer copies are the target goal.

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
         --send-json-header \
         --source-device eth0 \
         --debug

```

Build and run the zmq capture monitor client...

```text
# Use ENV Variable RUST_LOG= for logging level and cmdline args
RUST_LOG=info target/release/monitor \
         --source-ip 127.0.0.1 \
         --source-port 5556 \
         --recv-json-header \
         --debug

```

Check the output file capture.ts (or what you set in .env or environment variables)

```text
ffmpeg -i capture.ts
```

## Profiling with Intel Vtune (Linux/Windows)

```text
## Setup YUM Repo

# YUM add oneapi repo
tee > /tmp/oneAPI.repo << EOF
[oneAPI]
name=Intel® oneAPI repository
baseurl=https://yum.repos.intel.com/oneapi
enabled=1
gpgcheck=1
repo_gpgcheck=1
gpgkey=https://yum.repos.intel.com/intel-gpg-keys/GPG-PUB-KEY-INTEL-SW-PRODUCTS.PUB
EOF

# Copy YUM config for repo into place and install VTune
sudo mv /tmp/oneAPI.repo /etc/yum.repos.d
sudo yum install intel-oneapi-vtune
```

Running VTune [vtune.sh](vtune.sh)

```text
## Runtime for VTune
# Web UI (Best) Read [Intel VTune Documentation](https://www.intel.com/content/www/us/en/docs/vtune-profiler/user-guide/2024-0/web-server-ui.html)
./vtune.sh

# Command line (Optional, not preferred, doesn't work near as well with Rust at least)
source /opt/intel/oneapi/vtune/latest/vtune-vars.sh
/opt/intel/oneapi/vtune/latest/bin64/vtune \
        -collect performance-snapshot \
            target/debug/probe

/opt/intel/oneapi/vtune/latest/bin64/vtune \
        -collect hotspots \
        -result-dir results \
            target/release/probe

vtune -report summary -result-dir results -format html -report-output results/report.html
```

## TODO - roadmap plans

- (WIP) Add more information header to the stream data metadata like system stats, network stats, mediainfo, captions, ancillary data.
- (WIP) SMPTE 2110 handling reassembling frames and analogous to the MpegTS support.
- (WIP) PES parsing and analysis of streams.
- (WIP) FFmpeg libzmq protocol compatibility to allow branching off into libav easily.
- (WIP) Cap'n Proto for metadata sent through ZMQ to monitor modules. Replace all the JSON usage / remove overhead.
- (WIP) General network analyzer view of network around the streams we know/care about.
- Have multiple client modes to distribute processing of the stream on the zmq endpoints.
- Wrap [ltntstools](https://github.com/LTNGlobal-opensource/libltntstools) lib functionality into Rust through C bindings (If possible).
- SEI metadata decoding various aspects of MpegTS and VANC data from SMPTE2110.
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
- Caption packets and other SEI data / metadata extraction and sending.
- SMPTE2110 data stream and audio stream support (need to have more than one pcap ip/port and distinguish them apart).
- Meme like overlay of current frame and stream metrics on the thumbnail images with precise timing and frame information like a scope. (phone/pad usage)

### Chris Kennedy (C) 2023 LGPL

