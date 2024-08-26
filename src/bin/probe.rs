/*
 * rsprobe: probe.rs - MpegTS Stream Analysis Probe with Kafka and GStreamer
 *
 * Written in 2024 by Chris Kennedy (C)
 *
 * License: MIT
 *
 */

use ahash::AHashMap;
#[cfg(feature = "dpdk_enabled")]
use capsule::config::{load_config, DPDKConfig};
#[cfg(feature = "dpdk_enabled")]
use capsule::dpdk;
#[cfg(all(feature = "dpdk_enabled", target_os = "linux"))]
use capsule::prelude::*;
use clap::Parser;
use env_logger::{Builder as LogBuilder, Env};
use futures::stream::StreamExt;
use lazy_static::lazy_static;
use log::{debug, error, info};
use pcap::{Active, Capture, Device, PacketCodec};
use rsprobe::stream_data::process_mpegts_packet;
use rsprobe::stream_data::{
    cleanup_stale_streams, get_pid_map, identify_video_pid, parse_and_store_pat, process_packet,
    update_pid_map, Codec, PmtInfo, StreamData, PAT_PID,
};
use rsprobe::{current_unix_timestamp_ms, hexdump};
use serde::Serialize;
use serde_json::Value;
use std::sync::RwLock;
use std::{
    error::Error as StdError,
    fmt, io,
    io::Write,
    net::{IpAddr, Ipv4Addr, UdpSocket},
    sync::atomic::{AtomicBool, Ordering},
    sync::Arc,
    time::Instant,
};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{self};

lazy_static! {
    static ref PROBE_DATA: RwLock<AHashMap<String, ProbeData>> = RwLock::new(AHashMap::new());
}

struct ProbeData {
    stream_groupings: AHashMap<u16, StreamGrouping>,
    global_data: serde_json::Map<String, Value>,
}

struct StreamGrouping {
    stream_data_list: Vec<StreamData>,
}

#[derive(Serialize)]
struct PidStreamType {
    pid: u16,
    stream_type: String,
    stream_type_number: u8,
    media_type: String, // audio, video, data
    ccerrors: u32,
    bitrate: u64,
}

// Define your custom PacketCodec
pub struct BoxCodec;

impl PacketCodec for BoxCodec {
    type Item = (Box<[u8]>, std::time::SystemTime); // Adjusted to return SystemTime

    fn decode(&mut self, packet: pcap::Packet) -> Self::Item {
        // Convert pcap timestamp to SystemTime
        let timestamp = std::time::UNIX_EPOCH
            + std::time::Duration::new(
                packet.header.ts.tv_sec as u64,
                packet.header.ts.tv_usec as u32 * 1000,
            );
        (packet.data.into(), timestamp)
    }
}

// Define a custom error for when the target device is not found
#[derive(Debug)]
struct DeviceNotFoundError;

impl std::error::Error for DeviceNotFoundError {}

impl DeviceNotFoundError {
    #[allow(dead_code)]
    fn new() -> ErrorWrapper {
        ErrorWrapper(Box::new(Self))
    }
}

impl fmt::Display for DeviceNotFoundError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Target device not found")
    }
}

struct ErrorWrapper(Box<dyn StdError + Send + Sync>);

impl fmt::Debug for ErrorWrapper {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for ErrorWrapper {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl StdError for ErrorWrapper {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.0.source()
    }
}

pub trait Packet: Send {
    fn data(&self) -> &[u8];
}

// Common interface for DPDK functionality
trait DpdkPort: Send {
    fn start(&self) -> Result<(), Box<dyn std::error::Error>>;
    fn stop(&self) -> Result<(), Box<dyn std::error::Error>>;
    fn rx_burst(&self, packets: &mut Vec<Box<dyn Packet>>) -> Result<(), anyhow::Error>;
    // Other necessary methods...
}

// Implementation for Linux with DPDK enabled
#[cfg(all(feature = "dpdk_enabled", target_os = "linux"))]
struct RealDpdkPort(dpdk::Port);

#[cfg(all(feature = "dpdk_enabled", target_os = "linux"))]
impl DpdkPort for RealDpdkPort {
    fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.0.start()?;
        Ok(())
    }

    fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.0.stop()?;
        Ok(())
    }
    fn rx_burst(&self, packets: &mut Vec<Box<dyn Packet>>) -> Result<(), anyhow::Error> {
        // Logic for rx_burst...
        Ok(())
    }
}

#[cfg(all(feature = "dpdk_enabled", target_os = "linux"))]
fn init_dpdk(
    port_id: u16,
    promiscuous_mode: bool,
) -> Result<Box<dyn DpdkPort>, Box<dyn std::error::Error>> {
    // Initialize capsule environment
    let config = load_config()?;
    dpdk::eal::init(config)?;

    // Configure network interface
    let port = dpdk::Port::new(port_id)?;
    port.configure()?;

    // Set promiscuous mode if needed
    if promiscuous_mode {
        port.set_promiscuous(true)?;
    }

    // Start the port
    port.start()?;

    Ok(Box::new(RealDpdkPort(port)))
}

// Placeholder implementation for non-Linux or DPDK disabled builds
#[cfg(not(all(feature = "dpdk_enabled", target_os = "linux")))]
struct DummyDpdkPort;

#[cfg(not(all(feature = "dpdk_enabled", target_os = "linux")))]
impl DpdkPort for DummyDpdkPort {
    fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        Err("DPDK is not supported on this OS".into())
    }

    fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
        Err("DPDK is not supported on this OS".into())
    }

    fn rx_burst(&self, _packets: &mut Vec<Box<dyn Packet>>) -> Result<(), anyhow::Error> {
        Err(anyhow::Error::msg("DPDK is not supported on this OS"))
    }
}

#[cfg(not(all(feature = "dpdk_enabled", target_os = "linux")))]
fn init_dpdk(
    _port_id: u16,
    _promiscuous: bool,
) -> Result<Box<dyn DpdkPort>, Box<dyn std::error::Error>> {
    Ok(Box::new(DummyDpdkPort))
}

fn init_pcap(
    source_device: &str,
    #[cfg(target_os = "linux")] _use_wireless: bool,
    #[cfg(not(target_os = "linux"))] use_wireless: bool,
    promiscuous: bool,
    read_time_out: i32,
    read_size: i32,
    immediate_mode: bool,
    buffer_size: i64,
    source_protocol: &str,
    source_port: i32,
    source_ip: &str,
) -> Result<(Capture<Active>, UdpSocket), Box<dyn StdError>> {
    let devices = Device::list().map_err(|e| Box::new(e) as Box<dyn StdError>)?;
    debug!("init_pcap: devices: {:?}", devices);
    info!("init_pcap: specified source_device: {}", source_device);

    // Different handling for Linux and non-Linux systems
    #[cfg(target_os = "linux")]
    let target_device = devices
        .into_iter()
        .find(|d| d.name == source_device || source_device.is_empty())
        .ok_or_else(|| Box::new(DeviceNotFoundError) as Box<dyn StdError>)?;

    #[cfg(not(target_os = "linux"))]
    let target_device = devices
        .into_iter()
        .find(|d| {
            (d.name == source_device || source_device.is_empty())
                && d.flags.is_up()
                && !d.flags.is_loopback()
                && d.flags.is_running()
                && (!d.flags.is_wireless() || use_wireless)
        })
        .ok_or_else(|| Box::new(DeviceNotFoundError) as Box<dyn StdError>)?;

    // Get the IP address of the target device
    let interface_addr = target_device
        .addresses
        .iter()
        .find_map(|addr| match addr.addr {
            IpAddr::V4(ipv4_addr) => Some(ipv4_addr),
            _ => None,
        })
        .ok_or_else(|| "No valid IPv4 address found for target device")?;

    let multicast_addr = source_ip
        .parse::<Ipv4Addr>()
        .expect("Invalid IP address format for source_ip");

    info!(
        "init_pcap: UDP Socket Binding to interface {} with Join IGMP Multicast for address:port udp://{}:{}.",
        interface_addr, multicast_addr, source_port
    );

    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| Box::new(e) as Box<dyn StdError>)?;
    socket
        .join_multicast_v4(&multicast_addr, &interface_addr)
        .map_err(|e| Box::new(e) as Box<dyn StdError>)?;

    let source_host_and_port = format!(
        "{} dst port {} and ip dst host {}",
        source_protocol, source_port, source_ip
    );

    let cap = Capture::from_device(target_device.clone())
        .map_err(|e| Box::new(e) as Box<dyn StdError>)?
        .promisc(promiscuous)
        .timeout(read_time_out)
        .snaplen(read_size)
        .immediate_mode(immediate_mode)
        .buffer_size(buffer_size as i32)
        .open()
        .map_err(|e| Box::new(e) as Box<dyn StdError>)?;

    info!(
        "init_pcap: set non-blocking mode on capture device {}",
        target_device.name
    );

    let mut cap = cap
        .setnonblock()
        .map_err(|e| Box::new(e) as Box<dyn StdError>)?;

    info!(
        "init_pcap: set filter for {} on capture device {}",
        source_host_and_port, target_device.name
    );

    cap.filter(&source_host_and_port, true)
        .map_err(|e| Box::new(e) as Box<dyn StdError>)?;

    info!(
        "init_pcap: capture device {} successfully initialized",
        target_device.name
    );

    Ok((cap, socket))
}

/// RsProbe Configuration
#[derive(Parser, Debug)]
#[clap(
    author = "Chris Kennedy",
    version = "0.8.2",
    about = "MpegTS Stream Analysis Probe PID Mapper"
)]
struct Args {
    /// probe ID - ID for the probe to send with the messages
    #[clap(long, env = "PROBE_ID", default_value = "")]
    probe_id: String,

    /// Sets the batch size
    #[clap(long, env = "PCAP_BATCH_SIZE", default_value_t = 7)]
    pcap_batch_size: usize,

    /// Sets the payload offset
    #[clap(long, env = "PAYLOAD_OFFSET", default_value_t = 42)]
    payload_offset: usize,

    /// Sets the packet size
    #[clap(long, env = "PACKET_SIZE", default_value_t = 188)]
    packet_size: usize,

    /// Sets the read timeout
    #[clap(long, env = "READ_TIME_OUT", default_value_t = 300_000)]
    read_time_out: i32,

    /// Sets the source device
    #[clap(long, env = "SOURCE_DEVICE", default_value = "")]
    source_device: String,

    /// Sets the source IP
    #[clap(long, env = "SOURCE_IP", default_value = "224.0.0.200")]
    source_ip: String,

    /// Sets the source protocol
    #[clap(long, env = "SOURCE_PROTOCOL", default_value = "udp")]
    source_protocol: String,

    /// Sets the source port
    #[clap(long, env = "SOURCE_PORT", default_value_t = 10_000)]
    source_port: i32,

    /// Sets the debug mode
    #[clap(long, env = "DEBUG", default_value_t = false)]
    debug_on: bool,

    /// Sets the silent mode
    #[clap(long, env = "SILENT", default_value_t = false)]
    silent: bool,

    /// Sets if wireless is used
    #[clap(long, env = "USE_WIRELESS", default_value_t = false)]
    use_wireless: bool,

    /// number of packets to capture
    #[clap(long, env = "PACKET_COUNT", default_value_t = 0)]
    packet_count: u64,

    /// Turn off progress output dots
    #[clap(long, env = "NO_PROGRESS", default_value_t = false)]
    no_progress: bool,

    /// Use promiscuous mode
    #[clap(long, env = "PROMISCUOUS", default_value_t = false)]
    promiscuous: bool,

    /// Sets the pcap buffer size
    #[clap(long, env = "BUFFER_SIZE", default_value_t = 1 * 1_358 * 1_000)]
    buffer_size: usize,

    /// PCAP immediate mode
    #[clap(long, env = "IMMEDIATE_MODE", default_value_t = false)]
    immediate_mode: bool,

    /// PCAP output capture stats mode
    #[clap(long, env = "PCAP_STATS", default_value_t = false)]
    pcap_stats: bool,

    ///  MPSC Channel Size for ZeroMQ
    #[clap(long, env = "PCAP_CHANNEL_SIZE", default_value_t = 100000)]
    pcap_channel_size: usize,

    /// MPSC Channel Size for PCAP
    #[clap(long, env = "KAFKA_CHANNEL_SIZE", default_value_t = 100000)]
    kafka_channel_size: usize,

    /// DPDK enable
    #[clap(long, env = "DPDK", default_value_t = false)]
    dpdk: bool,

    /// DPDK Port ID
    #[clap(long, env = "DPDK_PORT_ID", default_value_t = 0)]
    dpdk_port_id: u16,

    /// IPC Path for ZeroMQ
    #[clap(long, env = "IPC_PATH")]
    ipc_path: Option<String>,

    /// Output file for Kafka
    #[clap(long, env = "OUTPUT_FILE", default_value = "")]
    output_file: String,

    /// Watch File - File we watch for changes to send as the streams.PID.log_line string
    #[clap(long, env = "WATCH_FILE", default_value = "")]
    watch_file: String,

    /// Input Codec - Expected codec type for Video stream, limited to h264, h265 or mpeg2.
    #[clap(long, env = "INPUT_CODEC", default_value = "h264")]
    input_codec: String,

    /// Loglevel - Log level for the application
    #[clap(long, env = "LOGLEVEL", default_value = "info")]
    loglevel: String,

    /// Kafka Broker
    #[clap(long, env = "KAFKA_BROKER", default_value = "")]
    kafka_broker: String,

    /// Kafka Topic
    #[clap(long, env = "KAFKA_TOPIC", default_value = "")]
    kafka_topic: String,

    /// Kafka timeout to drop packets
    #[clap(long, env = "KAFKA_TIMEOUT", default_value_t = 100)]
    kafka_timeout: u64,

    /// Kafka Key
    #[clap(long, env = "KAFKA_KEY", default_value = "")]
    kafka_key: String,

    /// Kafka sending interval in milliseconds
    #[clap(long, env = "KAFKA_INTERVAL", default_value_t = 1000)]
    kafka_interval: u64,

    /// Dump Packets - Dump packets to the console in hex
    #[clap(long, env = "DUMP_PACKETS", default_value_t = false)]
    dump_packets: bool,

    /// Remove stale streams - Clear any streams that have not been updated in the last N seconds
    #[clap(long, env = "REMOVE_STALE_STREAMS", default_value_t = 0)]
    remove_stale_streams: u64,
}

// MAIN Function
#[tokio::main]
async fn main() {
    let ctrl_c = tokio::signal::ctrl_c();
    let running = Arc::new(AtomicBool::new(true));

    tokio::select! {
        _ = ctrl_c => {
            println!("\nCtrl-C received, shutting down");
            running.store(false, Ordering::SeqCst);
        }
        _ = rsprobe(running.clone()) => {
            println!("\nRsProbe exited");
        }
    }
}

// RsProbeFunction
async fn rsprobe(running: Arc<AtomicBool>) {
    let running_capture = running.clone();
    let running_kafka = running.clone();

    dotenv::dotenv().ok(); // read .env file

    let args = Args::parse();

    // Clone the args for use in the threads
    let probe_id_clone = args.probe_id.clone();
    let source_ip_clone = args.source_ip.clone();
    let source_ip_clone1 = args.source_ip.clone();
    let source_ip_clone2 = args.source_ip.clone();

    #[cfg(all(feature = "dpdk_enabled", target_os = "linux"))]
    let use_dpdk = args.dpdk;

    println!("Starting RsProbe...");

    if args.silent {
        // set log level to error
        std::env::set_var("RUST_LOG", "error");
    }

    // calculate read size based on batch size and packet size
    let read_size: i32 =
        (args.packet_size as i32 * args.pcap_batch_size as i32) + args.payload_offset as i32; // pcap read size

    // Set Rust log level with --loglevel if it is set
    let loglevel = args.loglevel.to_lowercase();
    match loglevel.as_str() {
        "error" => {
            log::set_max_level(log::LevelFilter::Error);
        }
        "warn" => {
            log::set_max_level(log::LevelFilter::Warn);
        }
        "info" => {
            log::set_max_level(log::LevelFilter::Info);
        }
        "debug" => {
            log::set_max_level(log::LevelFilter::Debug);
        }
        "trace" => {
            log::set_max_level(log::LevelFilter::Trace);
        }
        _ => {
            log::set_max_level(log::LevelFilter::Info);
        }
    }

    // Initialize logging
    let env = Env::default().filter_or("RUST_LOG", loglevel.as_str()); // Default to `info` if `RUST_LOG` is not set
    LogBuilder::from_env(env).init();

    let (ptx, mut prx) = mpsc::channel::<(Arc<Vec<u8>>, u64, u64)>(args.pcap_channel_size);

    // Spawn a new thread for packet capture
    let capture_task = if cfg!(feature = "dpdk_enabled") && args.dpdk {
        // DPDK is enabled
        tokio::spawn(async move {
            let port_id = 0; // Set your port ID
            let promiscuous_mode = args.promiscuous;

            // Initialize DPDK
            let port = match init_dpdk(port_id, promiscuous_mode) {
                Ok(p) => p,
                Err(e) => {
                    error!("Failed to initialize DPDK: {:?}", e);
                    return;
                }
            };

            let mut packets = Vec::new();
            let mut last_iat = 0;
            while running_capture.load(Ordering::SeqCst) {
                match port.rx_burst(&mut packets) {
                    Ok(_) => {
                        for packet in packets.drain(..) {
                            // Extract data from the packet
                            let data = packet.data();

                            // Convert to Arc<Vec<u8>> to maintain consistency with pcap logic
                            let packet_data = Arc::new(data.to_vec());
                            let timestamp = current_unix_timestamp_ms().unwrap_or(0);
                            let iat = if last_iat == 0 {
                                0
                            } else {
                                timestamp - last_iat
                            };
                            last_iat = timestamp;

                            // Send packet data to processing channel
                            ptx.send((packet_data, timestamp, iat)).await.unwrap();

                            // Here you can implement additional processing such as parsing the packet,
                            // updating statistics, handling specific packet types, etc.
                        }
                    }
                    Err(e) => {
                        error!("Error fetching packets: {:?}", e);
                        break;
                    }
                }
            }

            // Cleanup
            // Handle stopping the port
            if let Err(e) = port.stop() {
                error!("Error stopping DPDK port: {:?}", e);
            }
        })
    } else {
        tokio::spawn(async move {
            // initialize the pcap
            let (cap, _socket) = init_pcap(
                &args.source_device,
                args.use_wireless,
                args.promiscuous,
                args.read_time_out,
                read_size,
                args.immediate_mode,
                args.buffer_size as i64,
                &args.source_protocol,
                args.source_port,
                &source_ip_clone,
            )
            .expect("Failed to initialize pcap");

            // Create a PacketStream from the Capture
            let mut stream = cap.stream(BoxCodec).unwrap();
            let mut count = 0;

            let mut stats_last_sent_ts = Instant::now();
            let mut packets_dropped = 0;
            let mut last_iat = 0;

            while running_capture.load(Ordering::SeqCst) {
                while let Some(packet) = stream.next().await {
                    if !running_capture.load(Ordering::SeqCst) {
                        break;
                    }
                    match packet {
                        Ok((data, system_time_timestamp)) => {
                            count += 1;
                            let packet_data = Arc::new(data.to_vec());

                            // Convert SystemTime to u64 milliseconds
                            let duration_since_epoch = system_time_timestamp
                                .duration_since(std::time::UNIX_EPOCH)
                                .expect("Time went backwards");
                            let timestamp_ms = duration_since_epoch.as_secs() * 1_000
                                + duration_since_epoch.subsec_millis() as u64;
                            let iat = if last_iat == 0 {
                                0
                            } else {
                                timestamp_ms - last_iat
                            };
                            last_iat = timestamp_ms;

                            match ptx.send((packet_data, timestamp_ms, iat)).await {
                                Ok(_) => {
                                    // Successfully sent, continue or perform other operations
                                }
                                Err(e) => {
                                    eprintln!("Error sending packet: {}", e);
                                    break; // Exit the loop if sending fails
                                }
                            }

                            if !running_capture.load(Ordering::SeqCst) {
                                break;
                            }
                            let current_ts = Instant::now();
                            if args.pcap_stats
                                && ((current_ts.duration_since(stats_last_sent_ts).as_secs() >= 30)
                                    || count == 1)
                            {
                                stats_last_sent_ts = current_ts;
                                let stats = stream.capture_mut().stats().unwrap();
                                println!(
                                    "[{}] #{} Current stats: Received: {}, Dropped: {}/{}, Interface Dropped: {} packet_size: {} bytes.",
                                    timestamp_ms, count, stats.received, stats.dropped - packets_dropped, stats.dropped, stats.if_dropped, data.len(),
                                );
                                packets_dropped = stats.dropped;
                            }
                        }
                        Err(e) => {
                            // Print error and information about it
                            error!("PCap Capture Error occurred: {}", e);
                            if e == pcap::Error::TimeoutExpired {
                                // If timeout expired, check for running_capture
                                if !running_capture.load(Ordering::SeqCst) {
                                    break;
                                }
                                // Timeout expired, continue and try again
                                continue;
                            } else {
                                // Exit the loop if an error occurs
                                running_capture.store(false, Ordering::SeqCst);
                                break;
                            }
                        }
                    }
                }
                if args.debug_on {
                    let stats = stream.capture_mut().stats().unwrap();
                    println!(
                        "Current stats: Received: {}, Dropped: {}, Interface Dropped: {}",
                        stats.received, stats.dropped, stats.if_dropped
                    );
                }
                if !running_capture.load(Ordering::SeqCst) {
                    break;
                }
            }

            let stats = stream.capture_mut().stats().unwrap();
            println!("Packet capture statistics:");
            println!("Received: {}", stats.received);
            println!("Dropped: {}", stats.dropped);
            println!("Interface Dropped: {}", stats.if_dropped);
        })
    };

    let mut probe_id = args.probe_id.clone();
    if probe_id_clone.is_empty() {
        // construct stream.source_ip and stream.source_port with stream.host
        probe_id = format!("{}:{}", source_ip_clone1, args.source_port);
    }
    let probe_id_clone2 = probe_id.clone();

    // Setup channel for passing stream_data for Kafka thread sending the stream data to monitor process
    let (ktx, mut krx) = mpsc::channel::<(Vec<Arc<StreamData>>, AHashMap<u16, Arc<StreamData>>)>(
        args.kafka_channel_size,
    );

    let kafka_broker_clone = args.kafka_broker.clone();
    let kafka_topic_clone = args.kafka_topic.clone();
    let kafka_topic_clone2 = args.kafka_topic.clone();
    let kafka_broker_clone2 = args.kafka_broker.clone();

    let collector_thread = tokio::spawn(async move {
        // exit thread if kafka_broker is not set
        if kafka_broker_clone.is_empty() || kafka_topic_clone.is_empty() {
            return;
        }

        let mut iat_max: u64 = 0;
        let mut iat_avg: u64 = 0;
        let mut iat_count: u64 = 0;

        while running_kafka.load(Ordering::SeqCst) {
            while let Some((mut batch, pid_map)) = krx.recv().await {
                debug!("Kafka received PID Map: {:#?}", pid_map);

                // create a single array structure for all the pids and stream_type_names and numbers to represent
                // the mpegts stream as an mpegts analyzer would see it
                let mut pid_stream_types = Vec::<PidStreamType>::new();
                for (pid, stream_data) in pid_map.iter() {
                    // use stream_type_number to determine the media type of audio, video padding, text or data
                    let media_type = match stream_data.stream_type_number {
                        0x00 => "padding",
                        0x01 => "video",
                        0x02 => "video",
                        0x03 => "audio",
                        0x04 => "audio",
                        0x0F => "audio",
                        0x10 => "video",
                        0x11 => "audio",
                        0x1A => "text",
                        0x1B => "video",
                        0x1C => "text",
                        0x1D => "video",
                        0x1E => "video",
                        0x1F => "video",
                        0x20 => "video",
                        0x21 => "video",
                        0x22 => "video",
                        0x25 => "video",
                        0x26 => "video",
                        0x27 => "video",
                        0x28 => "video",
                        0x29 => "video",
                        0x81 => "audio",
                        0x87 => "audio",
                        0x86 => "scte35",
                        0xFF => "padding",
                        _ => "data",
                    };

                    pid_stream_types.push(PidStreamType {
                        pid: *pid,
                        stream_type: stream_data.stream_type.clone(),
                        stream_type_number: stream_data.stream_type_number,
                        ccerrors: stream_data.error_count,
                        media_type: media_type.to_string(),
                        bitrate: stream_data.bitrate as u64,
                    });
                }

                // Process and send messages
                let batch_end = batch.len();
                let mut batch_pos = 0;
                for stream_data in batch.iter() {
                    batch_pos += 1;

                    let pid = stream_data.pid;
                    {
                        let mut probe_data_map = PROBE_DATA.write().unwrap();
                        if let Some(probe_data) = probe_data_map.get_mut(&probe_id_clone2.clone()) {
                            let stream_groupings = &mut probe_data.stream_groupings;
                            if let Some(grouping) = stream_groupings.get_mut(&pid) {
                                // Update the existing StreamData instance in the grouping
                                let last_stream_data =
                                    grouping.stream_data_list.last_mut().unwrap();
                                *last_stream_data = Arc::try_unwrap(stream_data.clone())
                                    .unwrap_or_else(|stream_data| (*stream_data).clone());
                            } else {
                                let new_grouping = StreamGrouping {
                                    stream_data_list: vec![Arc::try_unwrap(stream_data.clone())
                                        .unwrap_or_else(|stream_data| (*stream_data).clone())],
                                };
                                stream_groupings.insert(pid, new_grouping);
                            }
                        } else {
                            let mut new_stream_groupings = AHashMap::new();
                            let new_grouping = StreamGrouping {
                                stream_data_list: vec![Arc::try_unwrap(stream_data.clone())
                                    .unwrap_or_else(|stream_data| (*stream_data).clone())],
                            };
                            new_stream_groupings.insert(pid, new_grouping);
                            let new_probe_data = ProbeData {
                                stream_groupings: new_stream_groupings,
                                global_data: serde_json::Map::new(),
                            };
                            probe_data_map.insert(probe_id_clone2.clone(), new_probe_data);
                        }
                    }

                    // Create a new map to store the averaged probe data
                    let mut averaged_probe_data: serde_json::Map<String, serde_json::Value> =
                        serde_json::Map::new();

                    // Acquire write access to PROBE_DATA and Send to Kafka
                    {
                        let mut probe_data_map = PROBE_DATA.write().unwrap();

                        // Process each probe's data
                        for (_probe_id, probe_data) in probe_data_map.iter_mut() {
                            let stream_groupings = &probe_data.stream_groupings;
                            let mut flattened_data: serde_json::Map<String, Value> =
                                serde_json::Map::new();

                            // Initialize variables to accumulate global averages
                            let mut total_bitrate_avg: u64 = 0;
                            let mut total_cc_errors: u64 = 0;
                            let mut total_cc_errors_current: u64 = 0;
                            let mut stream_count: u64 = 0;
                            let mut source_ip: String = String::new();
                            let mut source_port: u32 = 0;
                            let mut current_iat_avg: u64 = 0;

                            // Process each stream to accumulate averages
                            for (_, grouping) in stream_groupings.iter() {
                                for stream_data in &grouping.stream_data_list {
                                    total_bitrate_avg += stream_data.bitrate_avg as u64;
                                    if stream_data.capture_iat > 0 {
                                        iat_avg += stream_data.capture_iat;
                                        current_iat_avg = stream_data.capture_iat;
                                        iat_count += 1;
                                    }
                                    if stream_data.capture_iat_max > 0
                                        && stream_data.capture_iat_max > iat_max
                                    {
                                        iat_max = stream_data.capture_iat_max;
                                    }
                                    total_cc_errors += stream_data.error_count as u64;
                                    total_cc_errors_current +=
                                        stream_data.current_error_count as u64;
                                    source_port = stream_data.source_port as u32;
                                    source_ip = stream_data.source_ip.clone();

                                    stream_count += 1;
                                }
                            }

                            // Continuity Counter errors
                            let global_cc_errors = total_cc_errors;
                            let global_cc_errors_current = total_cc_errors_current;

                            // long-running avg IAT
                            let global_iat_avg = if iat_count > 0 {
                                iat_avg as f64 / iat_count as f64
                            } else {
                                0.0
                            };

                            // current avg IAT
                            let global_current_iat_avg = if iat_count > 0 {
                                current_iat_avg as f64 / stream_count as f64
                            } else {
                                0.0
                            };

                            // Calculate global averages
                            let global_bitrate_avg = if stream_count > 0 {
                                total_bitrate_avg
                            } else {
                                0
                            };
                            let current_timestamp = current_unix_timestamp_ms().unwrap_or(0);

                            // Directly insert global statistics and timestamp into the flattened_data map
                            flattened_data.insert(
                                "bitrate_avg".to_string(),
                                serde_json::json!(global_bitrate_avg),
                            );
                            flattened_data
                                .insert("iat_avg".to_string(), serde_json::json!(global_iat_avg));

                            flattened_data.insert(
                                "iat".to_string(),
                                serde_json::json!(global_current_iat_avg),
                            );

                            flattened_data
                                .insert("iat_max".to_string(), serde_json::json!(iat_max));

                            flattened_data.insert(
                                "cc_errors".to_string(),
                                serde_json::json!(global_cc_errors),
                            );
                            flattened_data.insert(
                                "current_cc_errors".to_string(),
                                serde_json::json!(global_cc_errors_current),
                            );
                            flattened_data.insert(
                                "timestamp".to_string(),
                                serde_json::json!(current_timestamp),
                            );
                            flattened_data
                                .insert("source_ip".to_string(), serde_json::json!(source_ip));
                            flattened_data
                                .insert("source_port".to_string(), serde_json::json!(source_port));
                            flattened_data
                                .insert("id".to_string(), serde_json::json!(probe_id_clone2));
                            flattened_data
                                .insert("pid_map".to_string(), serde_json::json!(pid_stream_types));

                            // Define the possible media types
                            let media_types =
                                vec!["video", "audio", "text", "data", "padding", "scte35"];

                            // Initialize the bitrate_fields with default values of 0 for each media type
                            let mut bitrate_fields: AHashMap<String, u64> = media_types
                                .into_iter()
                                .map(|media_type| (format!("bitrate.{}", media_type), 0))
                                .collect();

                            // Update the bitrate_fields with the actual bitrate values from pid_stream_types
                            for pid_stream_type in pid_stream_types.iter() {
                                let media_type = &pid_stream_type.media_type;
                                let bitrate_field = format!("bitrate.{}", media_type);

                                let bitrate = bitrate_fields.entry(bitrate_field).or_insert(0);

                                *bitrate += pid_stream_type.bitrate;
                            }

                            // Insert the bitrate fields into the flattened_data
                            for (bitrate_field, bitrate) in bitrate_fields.into_iter() {
                                flattened_data.insert(bitrate_field, serde_json::json!(bitrate));
                            }

                            // Merge the probe-specific flattened data with the global data
                            flattened_data.extend(probe_data.global_data.clone());

                            // Store the flattened data in the averaged_probe_data map
                            averaged_probe_data.insert(
                                probe_id_clone2.clone(),
                                serde_json::Value::Object(flattened_data),
                            );

                            // Clear the global data after processing
                            probe_data.global_data.clear();
                        }
                    }

                    // Inside the loop
                    for (_probe_id, probe_data) in averaged_probe_data.iter() {
                        if batch_pos != batch_end {
                            continue;
                        }
                        // TODO: Add collector send here

                        let json_data = serde_json::to_string(probe_data)
                            .expect("Failed to serialize probe data for Kafka");

                        // print out json_data in a neat way to compare each output easily by humans
                        println!("{}", json_data);
                        println!("------------------------------------------------------");

                        /*tokio::spawn(async move {
                            let producer_local = create_kafka_producer(&kafka_conf_clone).await;
                            let timeout = Duration::from_secs(30);
                            let retry_attempts = 3;
                            let retry_delay = Duration::from_millis(100);

                            if let Err(e) = send_to_kafka(
                                &producer_local,
                                &kafka_topic_clone,
                                &kafka_key_clone,
                                &json_data,
                                timeout,
                                retry_attempts,
                                retry_delay,
                            )
                            .await
                            {
                                log::error!("Failed to send message to Kafka: {:?}", e);
                            }
                        });*/
                    }
                }
                batch.clear();
            }
            // Sleep for a short time to avoid busy waiting
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    });

    // start time
    let start_time = current_unix_timestamp_ms().unwrap_or(0);

    let mut packets_captured = 0;

    // Start packet capture
    let mut video_pid: Option<u16> = Some(0xFFFF);
    let mut video_codec: Option<Codec> = Some(Codec::NONE);
    let mut current_video_frame = Vec::<StreamData>::new();
    let mut pmt_info: PmtInfo = PmtInfo {
        pid: 0xFFFF,
        packet: Vec::new(),
        program_number: 0,
    };
    let mut pmt_pid: Option<u16> = Some(0xFFFF);
    let mut program_number: Option<u16> = Some(0xFFFF);

    let mut video_stream_type = 0;

    let ktx_clone1 = ktx.clone();
    let ktx_clone2 = ktx.clone();

    info!("RsProbe: Starting up with Probe ID: {}", probe_id.clone());

    let mut dot_last_sent_ts = Instant::now();
    let mut x_last_sent_ts = Instant::now();

    let mut last_kafka_send_time = Instant::now();

    loop {
        match prx.try_recv() {
            Ok((packet, timestamp, iat)) => {
                if args.packet_count > 0 && packets_captured > args.packet_count {
                    println!(
                        "\nPacket count limit reached {}/{}, signaling termination...",
                        packets_captured, args.packet_count
                    );
                    running.store(false, Ordering::SeqCst);
                    break;
                }
                packets_captured += 1;

                if !args.no_progress && dot_last_sent_ts.elapsed().as_secs() >= 1 {
                    dot_last_sent_ts = Instant::now();
                    print!(".");
                    // Flush stdout to ensure the progress dots are printed
                    io::stdout().flush().unwrap();
                }

                let chunks = process_mpegts_packet(
                    args.payload_offset,
                    packet,
                    args.packet_size,
                    start_time,
                    timestamp,
                    iat,
                    source_ip_clone2.clone(),
                    args.source_port,
                    probe_id.clone(),
                );

                for mut stream_data in chunks {
                    stream_data.pmt_pid = pmt_pid.expect("Failed to get PMT PID");
                    stream_data.program_number =
                        program_number.expect("Failed to get program number");

                    // Process the chunk
                    if args.dump_packets {
                        hexdump(
                            &stream_data.packet,
                            stream_data.packet_start,
                            stream_data.packet_len,
                        );
                    }
                    // Extract the necessary slice for PID extraction and parsing
                    let packet_chunk = &stream_data.packet[stream_data.packet_start
                        ..stream_data.packet_start + stream_data.packet_len];

                    let pid = stream_data.pid;
                    // Handle PAT and PMT packets
                    match pid {
                        PAT_PID => {
                            debug!("ProcessPacket: PAT packet detected with PID {}", pid);
                            pmt_info = parse_and_store_pat(&packet_chunk);
                        }
                        _ => {
                            // Check if this is a PMT packet
                            if pid == pmt_info.pid {
                                if pmt_pid == Some(0xFFFF) {
                                    debug!("ProcessPacket: PMT packet detected with PID {}", pid);
                                    stream_data.pmt_pid = pid;
                                    pmt_pid = Some(pid);
                                }
                                // Update PID_MAP with new stream types
                                if args.remove_stale_streams > 0 {
                                    cleanup_stale_streams(args.remove_stale_streams.clone());
                                }
                                let program_number_result = update_pid_map(
                                    &packet_chunk,
                                    &pmt_info.packet,
                                    stream_data.capture_time,
                                    stream_data.capture_iat,
                                    source_ip_clone2.clone(),
                                    args.source_port,
                                    probe_id_clone.clone(),
                                );
                                program_number = Some(program_number_result);

                                // Identify the video PID (if not already identified)
                                if let Some((new_pid, new_codec)) =
                                    identify_video_pid(&packet_chunk)
                                {
                                    if stream_data.stream_type_number > 0
                                        && video_pid.map_or(true, |vp| vp != new_pid)
                                    {
                                        video_pid = Some(new_pid);
                                        let old_stream_type = video_stream_type;
                                        video_stream_type = stream_data.stream_type_number;
                                        info!(
                                            "Found Video PID and Codec {}/{}/{} changed from {}/{}/{}.",
                                            new_pid,
                                            new_codec.clone(),
                                            video_stream_type,
                                            video_pid.unwrap(),
                                            video_codec.unwrap(),
                                            old_stream_type
                                        );
                                        video_codec = Some(new_codec.clone());
                                        // Reset video frame as the video stream has changed
                                        current_video_frame.clear();
                                    } else if video_codec != Some(new_codec.clone()) {
                                        info!(
                                            "[{}][{}] Video Codec Detected: [{}] changed from previous value [{}] stream type {}/{}.",
                                            stream_data.capture_time,
                                            stream_data.program_number,
                                            new_codec,
                                            video_codec.unwrap(),
                                            stream_data.stream_type_number,
                                            stream_data.stream_type
                                        );
                                        video_codec = Some(new_codec);
                                        // Reset video frame as the codec has changed
                                        current_video_frame.clear();
                                    }
                                }
                            }
                        }
                    }

                    if video_pid < Some(0x1FFF)
                        && video_pid > Some(0)
                        && stream_data.pid == video_pid.unwrap()
                        && video_stream_type != stream_data.stream_type_number
                    {
                        let old_stream_type = video_stream_type;
                        video_stream_type = stream_data.stream_type_number;
                        info!(
                            "Found Video PID/Stream Type {}/{} changed from previous {}/{}.",
                            video_pid.unwrap(),
                            video_stream_type,
                            video_pid.unwrap(),
                            old_stream_type
                        );
                    }

                    // Process the packet
                    process_packet(&mut stream_data, pmt_info.pid, probe_id_clone.clone());

                    // release the packet Arc so it can be reused
                    stream_data.packet = Arc::new(Vec::new()); // Create a new Arc<Vec<u8>> for the next packet
                    stream_data.packet_len = 0;
                    stream_data.packet_start = 0;

                    // Check if the batch size is reached or the batch timeout has elapsed
                    if kafka_broker_clone2.is_empty() || kafka_topic_clone2.is_empty() {
                        // If Kafka is not enabled, print the batch to stdout
                        // Print the batch to stdout
                        log::debug!("Stream data: {:?}", stream_data);
                    } else if last_kafka_send_time.elapsed().as_millis()
                        >= args.kafka_interval as u128
                    {
                        // get pid map and push each pid stream_data into the batch
                        let pid_map = get_pid_map();

                        // get all the pids and push each into the batch
                        let mut batch = Vec::new();
                        for (pid, pid_stream_data) in pid_map.iter() {
                            debug!(
                                "Got PID: {} stream type: {}",
                                pid, pid_stream_data.stream_type
                            );
                            // Add the processed stream_data to the batch
                            batch.push(Arc::clone(&pid_stream_data));
                        }

                        if ktx_clone1
                            .send((batch.clone(), pid_map.clone()))
                            .await
                            .is_err()
                        {
                            // If the channel is full, drop the batch and log a warning
                            log::warn!("Batch channel is full. Dropping batch.");
                        }
                        // Update last send time
                        last_kafka_send_time = Instant::now();
                    }
                }
            }
            Err(TryRecvError::Empty) => {
                // No packets received, print 'X' to indicate
                if !args.no_progress && x_last_sent_ts.elapsed().as_secs() >= 1 {
                    x_last_sent_ts = Instant::now();
                    print!("X");
                    // Flush stdout to ensure the progress dots are printed
                    io::stdout().flush().unwrap();
                }

                // Flush stdout to ensure the 'X' is printed
                io::stdout().flush().unwrap();
                // Sleep for a short duration to avoid high CPU usage
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
            Err(TryRecvError::Disconnected) => {
                // The channel has been disconnected, break the loop
                println!("\nChannel disconnected, terminating...");
                running.store(false, Ordering::SeqCst);
                break;
            }
        }
    }

    println!("\nSending stop signals to threads...");

    // Send Kafka stop signal
    let _ = ktx_clone2.try_send((Vec::new(), AHashMap::new()));
    drop(ktx_clone2);

    // Wait for the kafka thread to finish
    capture_task.await.unwrap();
    collector_thread.await.unwrap();

    println!("\nThreads finished, exiting rsprobe");
}
