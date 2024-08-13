# MpegTS Stream Analysis PID Mapper Probe in Rust

## Overview

Capture MpegTS off a network and analyze for broadcast monitoring purposes. Measure various properties and extract assets from a PCap-sourced MpegTS multicast network stream.

## Features

- MpegTS PCap stream capture and analysis off network UDP multicast feeds.

## The Probe Client

The [src/bin/probe.rs](src/bin/probe.rs) is the main entry point for the probe client. It captures the MpegTS stream and extracts metrics and assets from the stream. The probe client is the main entry point for the project and is the main executable for the project.

## Configuration with Environment Variables

Use `.env` and/or command line args to override the default/env variables. See [.env.example](.env.example) for an example configuration. The command line args are the same as the environment variables but with `--` prepended to the variable name and lower case.

## Building and Executing RsProbe with Dependencies

- RsProbe Install script: [scripts/install.sh](scripts/install.sh) for deps setup on Linux CentOS 7 and macOS into the `/opt/rsprobe` contained directory.
- RsProbe Compile script: [scripts/compile.sh](scripts/compile.sh) for RsProbe build setup in `/opt/rsprobe`.

```text
# Install RsProbe in /opt/rsprobe/ (MacOS or CentOS 7)
./scripts/install.sh # or make install

# Optionally rebuild RsProbe if making changes
./scripts/compile.sh

# Run the probe
./scripts/probe.sh -h # help to see what options are available
```

### Chris Kennedy (C) 2024 MIT License
