## Packet Capture MpegTS/SMPTE2110 for ZeroMQ Distributed Processing

Distribute an MpegTS network stream over ZeroMQ. Capture
the TS using pcap with filter rules for specifying
which stream. Validate the stream for conformance
keeping the ZeroMQ output clean without any non-legal
TS packets. Store metadata extracted in zeromq json headers.
Share out multicast to many clients for distributed stream processing.

![rscap](https://storage.googleapis.com/gaib/2/rscap/rscap.png)

## This consists of two programs, a probe and a monitor client.

- The probe takes MpegTS via Packet Capture and publishes
batches of the MpegTS 188 byte packets to a ZeroMQ output.

- The monitor client reads from the ZeroMQ socket and writes out a
a file containing the MpegTS stream matching the one
captured by the probe.

## Configuration with environment variables using [.env](.env.example)

Use .env and/or command line args to override the default/env variables.

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

On Linux update libpcap to the newest version (optional)...

```text
sudo yum -y install flex                                                                                                                                       |
sudo yum -y install bison byacc yacc

wget https://www.tcpdump.org/release/libpcap-1.10.4.tar.gz
tar xfz libpcap-1.10.4.tar.gz

cd libpcap-1.10.4
./configure --prefix=/usr --libdir=/usr/lib64 --includedir=/usr/include --exec_prefix=/usr
make
sudo make install
```

Build and run the pcap stream probe...

```text
# Compile release,release-with-debug, and release  versions in target/ directories.
[./compile.sh](compile.sh)

# Use ENV Variable RUST_LOG= for logging level and cmdline args
sudo RUST_LOG=info target/release/[probe](src/bin/probe.rs) \
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

```
tee > /tmp/oneAPI.repo << EOF
[oneAPI]
name=Intel® oneAPI repository
baseurl=https://yum.repos.intel.com/oneapi
enabled=1
gpgcheck=1
repo_gpgcheck=1
gpgkey=https://yum.repos.intel.com/intel-gpg-keys/GPG-PUB-KEY-INTEL-SW-PRODUCTS.PUB
EOF

sudo mv /tmp/oneAPI.repo /etc/yum.repos.d
sudo yum install intel-oneapi-vtune

# Web UI (Best) Read [Intel VTune Documentation](https://www.intel.com/content/www/us/en/docs/vtune-profiler/user-guide/2024-0/web-server-ui.html)
./vtune.sh

# Command line (Optional)
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
- Segmentation of captured MpegTS, VOD file writer by various specs.
- Compression for proxy capture. Encode bitrate ladders realtime in parallel?
- Multiple streams per probe? Seems better to separate each probe to avoid a mess.
- Audio analysis, capture, sampling.

### Chris Kennedy (C) 2023 LGPL

