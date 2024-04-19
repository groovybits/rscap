/*
 * rsprobe: probe.rs - Rust Stream Capture with pcap, output serialized stats to ZeroMQ
 *
 * Written in 2024 by Chris Kennedy (C)
 *
 * License: MIT
 *
 */

use ahash::AHashMap;
use base64::{engine::general_purpose, Engine as _};
#[cfg(feature = "dpdk_enabled")]
use capsule::config::{load_config, DPDKConfig};
#[cfg(feature = "dpdk_enabled")]
use capsule::dpdk;
#[cfg(all(feature = "dpdk_enabled", target_os = "linux"))]
use capsule::prelude::*;
use clap::Parser;
use env_logger::{Builder as LogBuilder, Env};
use futures::stream::StreamExt;
#[cfg(feature = "gst")]
use gstreamer as gst;
#[cfg(feature = "gst")]
use gstreamer::prelude::*;
use lazy_static::lazy_static;
use log::{debug, error, info};
use pcap::{Active, Capture, Device, PacketCodec};
use rdkafka::admin::{AdminClient, AdminOptions, NewTopic};
use rdkafka::client::DefaultClientContext;
use rdkafka::config::ClientConfig;
use rdkafka::producer::{FutureProducer, FutureRecord};
use rsprobe::stream_data::{
    identify_video_pid, is_mpegts_or_smpte2110, parse_and_store_pat, process_packet,
    update_pid_map, Codec, PmtInfo, StreamData, Tr101290Errors, PAT_PID,
};
#[cfg(feature = "gst")]
use rsprobe::stream_data::{initialize_pipeline, process_video_packets, pull_images};
use rsprobe::stream_data::{process_mpegts_packet, process_smpte2110_packet};
use rsprobe::watch_file::watch_daemon;
use rsprobe::{current_unix_timestamp_ms, hexdump};
use rsprobe::{get_system_stats, SystemStats};
use serde_json::{json, Value};
use std::fs::File;
use std::sync::mpsc::channel;
use std::sync::RwLock;
use std::thread;
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
use tokio::time::Duration;

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

// Callback function to check the status of the sent messages
async fn delivery_report(
    result: Result<(i32, i64), (rdkafka::error::KafkaError, rdkafka::message::OwnedMessage)>,
) {
    match result {
        Ok((partition, offset)) => log::debug!(
            "Message delivered to partition {} at offset {}",
            partition,
            offset
        ),
        Err((err, _)) => println!("Message delivery failed: {:?}", err),
    }
}

fn flatten_streams(
    stream_groupings: &AHashMap<u16, StreamGrouping>,
    system_stats: SystemStats,
) -> serde_json::Map<String, Value> {
    let mut flat_structure: serde_json::Map<String, Value> = serde_json::Map::new();

    for (pid, grouping) in stream_groupings.iter() {
        let stream_data = grouping.stream_data_list.last().unwrap(); // Assuming last item is representative

        let prefix = format!("streams.{}", pid);

        // Adding basic info of the stream
        flat_structure.insert(
            format!("{}.stream_type", prefix),
            json!(stream_data.stream_type),
        );
        flat_structure.insert(
            format!("{}.program_number", prefix),
            json!(stream_data.program_number),
        );
        flat_structure.insert(format!("{}.pmt_pid", prefix), json!(stream_data.pmt_pid));
        flat_structure.insert(
            format!("{}.bitrate_avg", prefix),
            json!(stream_data.bitrate_avg),
        );
        flat_structure.insert(format!("{}.iat_avg", prefix), json!(stream_data.iat_avg));
        flat_structure.insert(
            format!("{}.packet_count", prefix),
            json!(grouping.stream_data_list.len()),
        );

        // Adding detailed stats of the stream
        flat_structure.insert(format!("{}.pid", prefix), json!(stream_data.pid));
        flat_structure.insert(format!("{}.pmt_pid", prefix), json!(stream_data.pmt_pid));
        flat_structure.insert(
            format!("{}.program_number", prefix),
            json!(stream_data.program_number),
        );
        flat_structure.insert(
            format!("{}.stream_type", prefix),
            json!(stream_data.stream_type),
        );
        flat_structure.insert(
            format!("{}.continuity_counter", prefix),
            json!(stream_data.continuity_counter),
        );
        flat_structure.insert(
            format!("{}.timestamp", prefix),
            json!(stream_data.timestamp),
        );
        flat_structure.insert(format!("{}.bitrate", prefix), json!(stream_data.bitrate));
        flat_structure.insert(
            format!("{}.bitrate_max", prefix),
            json!(stream_data.bitrate_max),
        );
        flat_structure.insert(
            format!("{}.bitrate_min", prefix),
            json!(stream_data.bitrate_min),
        );
        flat_structure.insert(
            format!("{}.bitrate_avg", prefix),
            json!(stream_data.bitrate_avg),
        );
        flat_structure.insert(format!("{}.iat", prefix), json!(stream_data.iat));
        flat_structure.insert(format!("{}.iat_max", prefix), json!(stream_data.iat_max));
        flat_structure.insert(format!("{}.iat_min", prefix), json!(stream_data.iat_min));
        flat_structure.insert(format!("{}.iat_avg", prefix), json!(stream_data.iat_avg));
        flat_structure.insert(
            format!("{}.error_count", prefix),
            json!(stream_data.error_count),
        );
        flat_structure.insert(
            format!("{}.current_error_count", prefix),
            json!(stream_data.current_error_count),
        );
        flat_structure.insert(
            format!("{}.last_arrival_time", prefix),
            json!(stream_data.last_arrival_time),
        );
        flat_structure.insert(
            format!("{}.last_sample_time", prefix),
            json!(stream_data.last_sample_time),
        );
        flat_structure.insert(
            format!("{}.start_time", prefix),
            json!(stream_data.start_time),
        );
        flat_structure.insert(
            format!("{}.total_bits", prefix),
            json!(stream_data.total_bits),
        );
        flat_structure.insert(
            format!("{}.total_bits_sample", prefix),
            json!(stream_data.total_bits_sample),
        );
        flat_structure.insert(format!("{}.count", prefix), json!(stream_data.count));
        flat_structure.insert(
            format!("{}.packet_start", prefix),
            json!(stream_data.packet_start),
        );
        flat_structure.insert(
            format!("{}.packet_len", prefix),
            json!(stream_data.packet_len),
        );
        flat_structure.insert(
            format!("{}.rtp_timestamp", prefix),
            json!(stream_data.rtp_timestamp),
        );
        flat_structure.insert(
            format!("{}.rtp_payload_type", prefix),
            json!(stream_data.rtp_payload_type),
        );
        flat_structure.insert(
            format!("{}.rtp_payload_type_name", prefix),
            json!(stream_data.rtp_payload_type_name),
        );
        flat_structure.insert(
            format!("{}.rtp_line_number", prefix),
            json!(stream_data.rtp_line_number),
        );
        flat_structure.insert(
            format!("{}.rtp_line_offset", prefix),
            json!(stream_data.rtp_line_offset),
        );
        flat_structure.insert(
            format!("{}.rtp_line_length", prefix),
            json!(stream_data.rtp_line_length),
        );
        flat_structure.insert(
            format!("{}.rtp_field_id", prefix),
            json!(stream_data.rtp_field_id),
        );
        flat_structure.insert(
            format!("{}.rtp_line_continuation", prefix),
            json!(stream_data.rtp_line_continuation),
        );
        flat_structure.insert(
            format!("{}.rtp_extended_sequence_number", prefix),
            json!(stream_data.rtp_extended_sequence_number),
        );
        flat_structure.insert(
            format!("{}.stream_type_number", prefix),
            json!(stream_data.stream_type_number),
        );
        flat_structure.insert(
            format!("{}.capture_time", prefix),
            json!(stream_data.capture_time),
        );
        flat_structure.insert(
            format!("{}.capture_iat", prefix),
            json!(stream_data.capture_iat),
        );
        flat_structure.insert(
            format!("{}.capture_iat_max", prefix),
            json!(stream_data.capture_iat_max),
        );

        flat_structure.insert(
            format!("{}.has_image", prefix),
            json!(stream_data.has_image),
        );
        flat_structure.insert(
            format!("{}.image_pts", prefix),
            json!(stream_data.image_pts),
        );
        flat_structure.insert(
            format!("{}.log_message", prefix),
            json!(stream_data.log_message),
        );

        // Add system stats fields to the flattened structure
        flat_structure.insert(
            format!("{}.total_memory", prefix),
            json!(system_stats.total_memory),
        );
        flat_structure.insert(
            format!("{}.used_memory", prefix),
            json!(system_stats.used_memory),
        );
        flat_structure.insert(
            format!("{}.total_swap", prefix),
            json!(system_stats.total_swap),
        );
        flat_structure.insert(
            format!("{}.used_swap", prefix),
            json!(system_stats.used_swap),
        );
        flat_structure.insert(
            format!("{}.cpu_usage", prefix),
            json!(system_stats.cpu_usage),
        );
        flat_structure.insert(
            format!("{}.cpu_count", prefix),
            json!(system_stats.cpu_count),
        );
        flat_structure.insert(
            format!("{}.core_count", prefix),
            json!(system_stats.core_count),
        );
        flat_structure.insert(
            format!("{}.boot_time", prefix),
            json!(system_stats.boot_time),
        );
        flat_structure.insert(
            format!("{}.load_avg_one", prefix),
            json!(system_stats.load_avg.one),
        );
        flat_structure.insert(
            format!("{}.load_avg_five", prefix),
            json!(system_stats.load_avg.five),
        );
        flat_structure.insert(
            format!("{}.load_avg_fifteen", prefix),
            json!(system_stats.load_avg.fifteen),
        );
        flat_structure.insert(
            format!("{}.host_name", prefix),
            json!(system_stats.host_name),
        );
        flat_structure.insert(
            format!("{}.kernel_version", prefix),
            json!(system_stats.kernel_version),
        );
        flat_structure.insert(
            format!("{}.os_version", prefix),
            json!(system_stats.os_version),
        );

        flat_structure.insert(
            format!("{}.process_count", prefix),
            json!(system_stats.process_count),
        );
        flat_structure.insert(format!("{}.uptime", prefix), json!(system_stats.uptime));
        flat_structure.insert(
            format!("{}.system_name", prefix),
            json!(system_stats.system_name),
        );

        // Flatten the network stats and insert them into the structure
        for network in &system_stats.network_stats {
            flat_structure.insert(
                format!("{}.network.{}.received", prefix, network.name),
                json!(network.received),
            );
            flat_structure.insert(
                format!("{}.network.{}.transmitted", prefix, network.name),
                json!(network.transmitted),
            );
            flat_structure.insert(
                format!("{}.network.{}.packets_received", prefix, network.name),
                json!(network.packets_received),
            );
            flat_structure.insert(
                format!("{}.network.{}.packets_transmitted", prefix, network.name),
                json!(network.packets_transmitted),
            );
            flat_structure.insert(
                format!("{}.network.{}.errors_on_received", prefix, network.name),
                json!(network.errors_on_received),
            );
            flat_structure.insert(
                format!("{}.network.{}.errors_on_transmitted", prefix, network.name),
                json!(network.errors_on_transmitted),
            );
        }

        // Flatten the disk stats and insert them into the structure
        for (i, disk) in system_stats.disk_stats.iter().enumerate() {
            flat_structure.insert(
                format!("{}.disk_stats_{}_name", prefix, i),
                json!(disk.name),
            );
            flat_structure.insert(
                format!("{}.disk_stats_{}_total_space", prefix, i),
                json!(disk.total_space),
            );
            flat_structure.insert(
                format!("{}.disk_stats_{}_available_space", prefix, i),
                json!(disk.available_space),
            );
            flat_structure.insert(
                format!("{}.disk_stats_{}_is_removable", prefix, i),
                json!(disk.is_removable),
            );
        }

        // Flatten the processes and insert them into the structure as an array of strings
        let cpu_threshold = 5.0; // CPU usage threshold (in percentage)
        let ram_threshold = 100 * 1024 * 1024; // RAM usage threshold (in bytes)

        let processes: Vec<String> = system_stats
            .processes
            .iter()
            .filter(|process| {
                process.cpu_usage > cpu_threshold || process.memory > ram_threshold
            })
            .map(|process| {
                format!(
                    "{{\"name\":\"{}\",\"pid\":{},\"cpu_usage\":{},\"memory\":{},\"virtual_memory\":{},\"start_time\":{}}}",
                    process.name, process.pid, process.cpu_usage, process.memory, process.virtual_memory, process.start_time
                )
            })
            .collect();

        flat_structure.insert(format!("{}.processes", prefix), json!(processes));

        flat_structure.insert(format!("{}.id", prefix), json!(stream_data.probe_id));
        flat_structure.insert(format!("{}.captions", prefix), json!(stream_data.captions));
    }

    flat_structure
}

async fn kafka_produce_message(
    data: Vec<u8>,
    kafka_server: String,
    kafka_topic: String,
    kafka_timeout: u64,
    key: String,
    _stream_data_timestamp: i64,
    producer: FutureProducer,
    admin_client: &AdminClient<DefaultClientContext>,
) {
    log::debug!("Service {} sending message", kafka_topic);
    let kafka_topic = kafka_topic.replace(":", "_").replace(".", "_");

    // Metadata fetching is problematic, directly attempt to ensure the topic exists.
    // This code block tries to create the topic if it doesn't already exist, ignoring errors that indicate existence.
    let new_topic = NewTopic::new(&kafka_topic, 1, rdkafka::admin::TopicReplication::Fixed(1));
    let _ = admin_client
        .create_topics(&[new_topic], &AdminOptions::new())
        .await;

    log::debug!(
        "Forwarding message for topic {} to Kafka server {:?}",
        kafka_topic,
        kafka_server
    );

    let record = FutureRecord::to(&kafka_topic).payload(&data).key(&key);
    /* .timestamp(stream_data_timestamp);*/

    let delivery_future = producer
        .send(record, Duration::from_secs(kafka_timeout))
        .await;
    match delivery_future {
        Ok(delivery_result) => delivery_report(Ok(delivery_result)).await,
        Err(e) => log::error!("Failed to send message: {:?}", e),
    }
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
    version = "0.5.101",
    about = "MpegTS Stream Analysis Probe with Kafka and GStreamer"
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

    /// Force smpte2110 mode
    #[clap(long, env = "SMPT2110", default_value_t = false)]
    smpte2110: bool,

    /// Use promiscuous mode
    #[clap(long, env = "PROMISCUOUS", default_value_t = false)]
    promiscuous: bool,

    /// Show the TR101290 p1, p2 and p3 errors if any
    #[clap(long, env = "SHOW_TR101290", default_value_t = false)]
    show_tr101290: bool,

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

    /// Sets the batch size
    #[clap(long, env = "KAFKA_BATCH_SIZE", default_value_t = 100)]
    kafka_batch_size: usize,

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

    /// Debug SMPTE2110
    #[clap(long, env = "DEBUG_SMPTE2110", default_value_t = false)]
    debug_smpte2110: bool,

    /// Watch File - File we watch for changes to send as the streams.PID.log_line string
    #[clap(long, env = "WATCH_FILE", default_value = "")]
    watch_file: String,

    /// Input Codec - Expected codec type for Video stream, limited to h264, h265 or mpeg2.
    #[clap(long, env = "INPUT_CODEC", default_value = "h264")]
    input_codec: String,

    /// Loglevel - Log level for the application
    #[clap(long, env = "LOGLEVEL", default_value = "info")]
    loglevel: String,

    /// Extract Images from the video stream (requires feature gst)
    #[clap(long, env = "EXTRACT_IMAGES", default_value_t = false)]
    extract_images: bool,

    /// Save Images to disk
    #[cfg(feature = "gst")]
    #[clap(long, env = "SAVE_IMAGES", default_value_t = false)]
    save_images: bool,

    /// Image Sample Rate Ns - Image sample rate in nano seconds (fails to get images as frequently)
    #[clap(long, env = "IMAGE_SAMPLE_RATE_NS", default_value_t = 0)]
    image_sample_rate_ns: u64,

    /// Scale Images using gstreamer - Scale the images with gstreamer instead of separately
    #[clap(long, env = "SCALE_IMAGES_AFTER_GSTREAMER", default_value_t = false)]
    scale_images_after_gstreamer: bool,

    /// Jpeg Quality - Quality of the Jpeg images
    #[clap(long, env = "JPEG_QUALITY", default_value_t = 75)]
    jpeg_quality: u8,

    /// Image Height - Image height in pixels of Thumbnail extracted images
    #[clap(long, env = "IMAGE_HEIGHT", default_value_t = 96)]
    image_height: u32,

    /// filmstrip length
    #[clap(long, env = "FILMSTRIP_LENGTH", default_value_t = 8)]
    filmstrip_length: usize,

    /// Gstreamer Queue Buffers
    #[clap(long, env = "GST_QUEUE_BUFFERS", default_value_t = 2)]
    gst_queue_buffers: u32,

    /// image framerate - Framerate of the images extracted in 1/1 format
    #[clap(long, env = "IMAGE_FRAMERATE", default_value = "1/1")]
    image_framerate: String,

    /// image_frame_increment - Increment the frame number by this amount for jpeg image strip, 0 matches filmstrip-length
    #[clap(long, env = "IMAGE_FRAME_INCREMENT", default_value_t = 1)]
    image_frame_increment: u8,

    /// Image buffer size - Size of the buffer for the images from gstreamer
    #[clap(long, env = "IMAGE_BUFFER_SIZE", default_value_t = 10)]
    image_buffer_size: usize,

    /// Video buffer size - Size of the buffer for the video to gstreamer
    #[clap(long, env = "VIDEO_BUFFER_SIZE", default_value_t = 100000)]
    video_buffer_size: usize,

    /// Chunk Buffer size = Size of the buffer for the chunked packet bundles to the main processing thread
    #[clap(long, env = "CHUNK_BUFFER_SIZE", default_value_t = 100000)]
    chunk_buffer_size: usize,

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
    #[cfg(feature = "gst")]
    let running_gstreamer_process = running.clone();
    #[cfg(feature = "gst")]
    let running_gstreamer_pull = running.clone();
    let running_watch_file = running.clone();

    dotenv::dotenv().ok(); // read .env file

    let args = Args::parse();

    // Use the parsed arguments directly
    let mut pcap_batch_size = args.pcap_batch_size;
    let payload_offset = args.payload_offset;
    let mut packet_size = args.packet_size;
    let read_time_out = args.read_time_out;
    let source_device = args.source_device;
    let source_ip = args.source_ip.clone();
    let source_protocol = args.source_protocol;
    let source_port = args.source_port;
    let debug_on = args.debug_on;
    let silent = args.silent;
    let use_wireless = args.use_wireless;
    let packet_count = args.packet_count;
    let no_progress = args.no_progress;
    let promiscuous = args.promiscuous;
    let show_tr101290 = args.show_tr101290;
    let mut buffer_size = args.buffer_size as i64;
    let mut immediate_mode = args.immediate_mode;
    let pcap_stats = args.pcap_stats;
    let mut pcap_channel_size = args.pcap_channel_size;
    let mut kafka_channel_size = args.kafka_channel_size;
    #[cfg(all(feature = "dpdk_enabled", target_os = "linux"))]
    let use_dpdk = args.dpdk;

    println!("Starting RsProbe...");

    // SMPTE2110 specific settings
    if args.smpte2110 {
        immediate_mode = true; // set immediate mode to true for smpte2110
        buffer_size = 10_000_000_000; // set pcap buffer size to 10GB for smpte2110
        pcap_channel_size = 1_000_000; // set pcap channel size for smpte2110
        kafka_channel_size = 1_000_000; // set kafka channel size for smpte2110
        packet_size = 1_220; // set packet size to 1220 (body) + 12 (header) for RTP
        pcap_batch_size = 3; // N x 1220 size packets for pcap read size
    }

    if silent {
        // set log level to error
        std::env::set_var("RUST_LOG", "error");
    }

    // calculate read size based on batch size and packet size
    let read_size: i32 = (packet_size as i32 * pcap_batch_size as i32) + payload_offset as i32; // pcap read size

    let mut is_mpegts = true; // Default to true, update based on actual packet type

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

    let (ptx, mut prx) = mpsc::channel::<(Arc<Vec<u8>>, u64, u64)>(pcap_channel_size);

    let source_ip_clone = source_ip.clone();

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
                &source_device,
                use_wireless,
                promiscuous,
                read_time_out,
                read_size,
                immediate_mode,
                buffer_size as i64,
                &source_protocol,
                source_port,
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
                            if pcap_stats
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
                if debug_on {
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

    // Setup channel for passing stream_data for Kafka thread sending the stream data to monitor process
    let (ktx, mut krx) = mpsc::channel::<Vec<StreamData>>(kafka_channel_size);

    let kafka_thread = tokio::spawn(async move {
        // exit thread if kafka_broker is not set
        if args.kafka_broker.is_empty() || args.kafka_topic.is_empty() {
            return;
        }

        let mut output_file_counter: u32 = 0;
        let mut last_kafka_send_time = Instant::now();
        let mut last_system_stats = Instant::now();
        let mut dot_last_file_write = Instant::now();
        let mut log_messages = Vec::<String>::new();
        let mut base64_image = String::new();
        let output_file_without_jpg = args.output_file.replace(".jpg", "");

        info!("Kafka publisher startup {}", args.kafka_broker);
        let mut kafka_conf = ClientConfig::new();
        kafka_conf.set("bootstrap.servers", &args.kafka_broker);
        kafka_conf.set("client.id", "rsprobe");

        let admin_client: AdminClient<DefaultClientContext> =
            kafka_conf.create().expect("Failed to create admin client");
        let producer: FutureProducer = kafka_conf
            .create()
            .expect("Failed to create Kafka producer");

        let mut system_stats = get_system_stats();
        while running_kafka.load(Ordering::SeqCst) {
            while let Some(mut batch) = krx.recv().await {
                // Process and send messages
                for stream_data in batch.iter() {
                    let mut force_send_message = false;

                    if stream_data.packet_len > 0 && stream_data.has_image > 0 {
                        let output_file_incremental =
                            format!("{}_{:08}.jpg", output_file_without_jpg, output_file_counter);

                        output_file_counter += 1;

                        let mut output_file_mut = if !args.output_file.is_empty() {
                            Some(File::create(&output_file_incremental).unwrap())
                        } else {
                            None
                        };
                        log::info!(
                            "Kafka Sender: [{}] Jpeg image received: {} size {} pts saved to {}",
                            stream_data.probe_id,
                            stream_data.packet_len,
                            stream_data.image_pts,
                            output_file_incremental
                        );

                        // Write to file if output_file is provided
                        if let Some(file) = output_file_mut.as_mut() {
                            if !no_progress && dot_last_file_write.elapsed().as_secs() > 1 {
                                dot_last_file_write = Instant::now();
                                print!("*");
                                // flush stdout
                                std::io::stdout().flush().unwrap();
                            }
                            file.write_all(&stream_data.packet).unwrap();
                        }

                        // Encode the JPEG image as Base64
                        base64_image =
                            general_purpose::STANDARD.encode(&stream_data.packet.as_ref());
                    }

                    let mut probe_id = stream_data.probe_id.clone();
                    if probe_id.is_empty() {
                        // construct stream.source_ip and stream.source_port with stream.host
                        let system_stats = get_system_stats();
                        probe_id = format!(
                            "{}:{}:{}",
                            system_stats.host_name, args.source_ip, args.source_port
                        );
                    }
                    let pid = stream_data.pid;
                    {
                        let mut probe_data_map = PROBE_DATA.write().unwrap();
                        if let Some(probe_data) = probe_data_map.get_mut(&probe_id) {
                            let stream_groupings = &mut probe_data.stream_groupings;
                            if let Some(grouping) = stream_groupings.get_mut(&pid) {
                                // Update the existing StreamData instance in the grouping
                                let last_stream_data =
                                    grouping.stream_data_list.last_mut().unwrap();
                                *last_stream_data = stream_data.clone();
                            } else {
                                let new_grouping = StreamGrouping {
                                    stream_data_list: vec![stream_data.clone()],
                                };
                                stream_groupings.insert(pid, new_grouping);
                            }
                        } else {
                            let mut new_stream_groupings = AHashMap::new();
                            let new_grouping = StreamGrouping {
                                stream_data_list: vec![stream_data.clone()],
                            };
                            new_stream_groupings.insert(pid, new_grouping);
                            let new_probe_data = ProbeData {
                                stream_groupings: new_stream_groupings,
                                global_data: serde_json::Map::new(),
                            };
                            probe_data_map.insert(probe_id.clone(), new_probe_data);
                        }
                    }

                    // Create a new map to store the averaged probe data
                    let mut averaged_probe_data: serde_json::Map<String, serde_json::Value> =
                        serde_json::Map::new();

                    //let packet_slice = &stream_data.packet[stream_data.packet_start
                    //    ..stream_data.packet_start + stream_data.packet_len];

                    // Check if it's time to send data to Kafka based on the interval
                    if !args.kafka_broker.is_empty() && args.kafka_broker != "" {
                        // Acquire write access to PROBE_DATA
                        {
                            let mut probe_data_map = PROBE_DATA.write().unwrap();
                            if last_system_stats.elapsed().as_millis() >= 1000 as u128 {
                                last_system_stats = Instant::now();
                                system_stats = get_system_stats();
                            }

                            // Process each probe's data
                            for (_probe_id, probe_data) in probe_data_map.iter_mut() {
                                let stream_groupings = &probe_data.stream_groupings;
                                let mut flattened_data =
                                    flatten_streams(&stream_groupings, system_stats.clone());

                                // Initialize variables to accumulate global averages
                                let mut total_bitrate_avg: u64 = 0;
                                let mut total_iat_avg: u64 = 0;
                                let mut total_iat_max: u64 = 0;
                                let mut total_cc_errors: u64 = 0;
                                let mut total_cc_errors_current: u64 = 0;
                                let mut stream_count: u64 = 0;
                                let mut source_ip: String = String::new();
                                let mut source_port: u32 = 0;
                                let mut image_pts: u64 = 0;
                                let mut captions: String = String::new();
                                let mut pid_map: String = String::new();
                                let mut scte35: String = String::new();
                                let mut audio_loudness: String = String::new();

                                // Process each stream to accumulate averages
                                for (_, grouping) in stream_groupings.iter() {
                                    for stream_data in &grouping.stream_data_list {
                                        total_bitrate_avg += stream_data.bitrate_avg as u64;
                                        total_iat_avg += stream_data.capture_iat;
                                        total_iat_max += stream_data.capture_iat_max;
                                        total_cc_errors += stream_data.error_count as u64;
                                        total_cc_errors_current +=
                                            stream_data.current_error_count as u64;
                                        source_port = stream_data.source_port as u32;
                                        source_ip = stream_data.source_ip.clone();
                                        if stream_data.has_image > 0 && stream_data.image_pts > 0 {
                                            image_pts = stream_data.image_pts;
                                        }
                                        if stream_data.log_message != "" {
                                            log::info!(
                                                "Got Log Message: {}",
                                                stream_data.log_message
                                            );
                                            log_messages.push(stream_data.log_message.clone());
                                        }
                                        if stream_data.captions != "" {
                                            // concatenate captions
                                            captions =
                                                format!("{}{}", captions, stream_data.captions);
                                        }
                                        if stream_data.pid_map != "" {
                                            // concatenate pid_map
                                            pid_map = format!("{}{}", pid_map, stream_data.pid_map);
                                        }
                                        if stream_data.scte35 != "" {
                                            // concatenate scte35
                                            scte35 = format!("{}{}", scte35, stream_data.scte35);
                                        }
                                        if stream_data.audio_loudness != "" {
                                            // concatenate audio_loudness
                                            audio_loudness = format!(
                                                "{}{}",
                                                audio_loudness, stream_data.audio_loudness
                                            );
                                        }
                                        stream_count += 1;
                                    }
                                }

                                // Continuity Counter errors
                                let global_cc_errors = total_cc_errors;
                                let global_cc_errors_current = total_cc_errors_current;

                                // avg IAT
                                let global_iat_avg = if stream_count > 0 {
                                    total_iat_avg as f64 / stream_count as f64
                                } else {
                                    0.0
                                };

                                // max IAT
                                let global_iat_max = if stream_count > 0 {
                                    total_iat_max as f64 / stream_count as f64
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
                                    "bitrate_avg_global".to_string(),
                                    serde_json::json!(global_bitrate_avg),
                                );
                                flattened_data.insert(
                                    "iat_avg_global".to_string(),
                                    serde_json::json!(global_iat_avg),
                                );
                                flattened_data.insert(
                                    "iat_max_global".to_string(),
                                    serde_json::json!(global_iat_max),
                                );
                                flattened_data.insert(
                                    "cc_errors_global".to_string(),
                                    serde_json::json!(global_cc_errors),
                                );
                                flattened_data.insert(
                                    "current_cc_errors_global".to_string(),
                                    serde_json::json!(global_cc_errors_current),
                                );
                                flattened_data.insert(
                                    "timestamp".to_string(),
                                    serde_json::json!(current_timestamp),
                                );
                                flattened_data
                                    .insert("source_ip".to_string(), serde_json::json!(source_ip));
                                flattened_data.insert(
                                    "source_port".to_string(),
                                    serde_json::json!(source_port),
                                );

                                if captions != "" {
                                    force_send_message = true;
                                }
                                flattened_data
                                    .insert("captions".to_string(), serde_json::json!(captions));

                                // Insert the base64_image field into the flattened_data map
                                flattened_data
                                    .insert("image_pts".to_string(), serde_json::json!(image_pts));
                                let base64_image_tag = if !base64_image.is_empty() {
                                    log::debug!("Got Image: {} bytes", base64_image.len());
                                    force_send_message = true;
                                    format!("data:image/jpeg;base64,{}", base64_image)
                                } else {
                                    "".to_string()
                                };
                                flattened_data.insert(
                                    "base64_image".to_string(),
                                    serde_json::json!(base64_image_tag),
                                );

                                // Check if we have a log_message in log_messages Vector, if so add it to the flattened_data map
                                if !log_messages.is_empty() {
                                    force_send_message = true;
                                    // remove one log message from the log_messages array
                                    let log_message = log_messages.pop().unwrap();
                                    flattened_data.insert(
                                        "log_message".to_string(),
                                        serde_json::json!(log_message),
                                    );
                                } else {
                                    flattened_data
                                        .insert("log_message".to_string(), serde_json::json!(""));
                                }

                                flattened_data
                                    .insert("id".to_string(), serde_json::json!(probe_id));
                                flattened_data
                                    .insert("pid_map".to_string(), serde_json::json!(pid_map));
                                flattened_data
                                    .insert("scte35".to_string(), serde_json::json!(scte35));
                                flattened_data.insert(
                                    "audio_loudness".to_string(),
                                    serde_json::json!(audio_loudness),
                                );

                                // Merge the probe-specific flattened data with the global data
                                flattened_data.extend(probe_data.global_data.clone());

                                // Store the flattened data in the averaged_probe_data map
                                averaged_probe_data.insert(
                                    probe_id.clone(),
                                    serde_json::Value::Object(flattened_data),
                                );

                                // Clear the global data after processing
                                probe_data.global_data.clear();
                            }
                        }

                        // Check if it's time to send data to Kafka based on the interval or if force_send_message is true
                        if force_send_message
                            || last_kafka_send_time.elapsed().as_millis()
                                >= args.kafka_interval as u128
                        {
                            for (_probe_id, probe_data) in averaged_probe_data.iter() {
                                let json_data = serde_json::to_value(probe_data)
                                    .expect("Failed to serialize probe data for Kafka");

                                let ser_data = serde_json::to_vec(&json_data)
                                    .expect("Failed to serialize json data for Kafka");

                                // Produce the message to Kafka
                                let future = kafka_produce_message(
                                    ser_data,
                                    args.kafka_broker.clone(),
                                    args.kafka_topic.clone(),
                                    args.kafka_timeout.clone(),
                                    args.kafka_key.clone(),
                                    current_unix_timestamp_ms().unwrap_or(0) as i64,
                                    producer.clone(),
                                    &admin_client,
                                );

                                future.await;
                            }

                            // Update last send time
                            last_kafka_send_time = Instant::now();
                        }
                    }
                }
                batch.clear();
            }
            // Sleep for a short time to avoid busy waiting
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    });

    // Create channels for sending video packets and receiving images
    #[cfg(feature = "gst")]
    let (video_packet_sender, video_packet_receiver) = mpsc::channel(args.video_buffer_size);
    #[cfg(feature = "gst")]
    let (image_sender, mut image_receiver) = mpsc::channel(args.image_buffer_size);

    // Initialize the pipeline
    #[cfg(feature = "gst")]
    let (pipeline, appsrc, appsink) = match initialize_pipeline(
        &args.input_codec,
        args.image_height,
        args.gst_queue_buffers,
        !args.scale_images_after_gstreamer,
        &args.image_framerate,
    ) {
        Ok((pipeline, appsrc, appsink)) => (pipeline, appsrc, appsink),
        Err(err) => {
            eprintln!("Failed to initialize the pipeline: {}", err);
            return;
        }
    };

    // Start the pipeline
    #[cfg(feature = "gst")]
    match pipeline.set_state(gst::State::Playing) {
        Ok(_) => (),
        Err(err) => {
            eprintln!("Failed to set the pipeline state to Playing: {}", err);
            return;
        }
    }

    // Spawn separate tasks for processing video packets and pulling images
    #[cfg(feature = "gst")]
    process_video_packets(
        appsrc,
        video_packet_receiver,
        running_gstreamer_process.clone(),
    );
    #[cfg(feature = "gst")]
    pull_images(
        appsink,
        /*Arc::new(Mutex::new(image_sender)),*/
        image_sender,
        args.save_images,
        args.image_sample_rate_ns,
        args.image_height,
        args.filmstrip_length,
        args.jpeg_quality,
        args.image_frame_increment,
        running_gstreamer_pull,
    );

    // Watch file thread and sender/receiver for log file input
    let (watch_file_sender, watch_file_receiver) = channel();
    let watch_file_sender_clone = watch_file_sender.clone();

    if args.watch_file != "" {
        let watch_file_clone = args.watch_file.clone();
        thread::spawn(move || {
            watch_daemon(
                &watch_file_clone,
                watch_file_sender_clone,
                running_watch_file,
            );
        });
    } else {
        info!("No watch file provided, skipping watch file thread.");
    }

    // Perform TR 101 290 checks
    let mut tr101290_errors = Tr101290Errors::new();

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
    };

    let mut video_stream_type = 0;

    let (chunk_sender, mut chunk_receiver) =
        tokio::sync::mpsc::channel::<Vec<StreamData>>(args.chunk_buffer_size);
    let probe_id_clone = args.probe_id.clone();
    let source_ip_clone_batch = source_ip.clone();

    let ktx_clone1 = ktx.clone();
    let ktx_clone2 = ktx.clone();

    // Create a separate thread for processing chunks
    tokio::spawn(async move {
        let mut batch = Vec::new();
        let batch_timeout = tokio::time::Duration::from_millis(1); // Adjust the batch timeout as needed
        let mut last_batch_time = tokio::time::Instant::now();

        while let Some(stream_data_chunk) = chunk_receiver.recv().await {
            for mut stream_data in stream_data_chunk {
                // Process the chunk
                if debug_on {
                    hexdump(
                        &stream_data.packet,
                        stream_data.packet_start,
                        stream_data.packet_len,
                    );
                }

                // Extract the necessary slice for PID extraction and parsing
                let packet_chunk = &stream_data.packet
                    [stream_data.packet_start..stream_data.packet_start + stream_data.packet_len];

                if is_mpegts {
                    let pid = stream_data.pid;
                    // Handle PAT and PMT packets
                    match pid {
                        PAT_PID => {
                            debug!("ProcessPacket: PAT packet detected with PID {}", pid);
                            pmt_info = parse_and_store_pat(&packet_chunk);
                            // Print TR 101 290 errors
                            if show_tr101290 {
                                info!("STATUS::TR101290:ERRORS: {}", tr101290_errors);
                            }
                        }
                        _ => {
                            // Check if this is a PMT packet
                            if pid == pmt_info.pid {
                                debug!("ProcessPacket: PMT packet detected with PID {}", pid);
                                // Update PID_MAP with new stream types
                                update_pid_map(
                                    &packet_chunk,
                                    &pmt_info.packet,
                                    stream_data.capture_time,
                                    stream_data.capture_iat,
                                    source_ip_clone_batch.clone(),
                                    args.source_port,
                                    probe_id_clone.clone(),
                                );
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
                                            "STATUS::VIDEO_PID:CHANGE: to {}/{}/{} from {}/{}/{}",
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
                                            "STATUS::VIDEO_CODEC:CHANGE: to {} from {}",
                                            new_codec,
                                            video_codec.unwrap()
                                        );
                                        video_codec = Some(new_codec);
                                        // Reset video frame as the codec has changed
                                        current_video_frame.clear();
                                    }
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
                        "STATUS::VIDEO_STREAM:FOUND: to {}/{} from {}/{}",
                        video_pid.unwrap(),
                        video_stream_type,
                        video_pid.unwrap(),
                        old_stream_type
                    );
                }

                // Check for TR 101 290 errors
                process_packet(
                    &mut stream_data,
                    &mut tr101290_errors,
                    is_mpegts,
                    pmt_info.pid,
                    probe_id_clone.clone(),
                );

                // If MpegTS, Check if this is a video PID and if so parse NALS and decode video
                if is_mpegts {
                    // Process video packets
                    #[cfg(feature = "gst")]
                    if args.extract_images {
                        #[cfg(feature = "gst")]
                        let video_packet = Arc::new(
                            stream_data.packet[stream_data.packet_start
                                ..stream_data.packet_start + stream_data.packet_len]
                                .to_vec(),
                        );

                        // Send the video packet to the processing task
                        if let Err(_) = video_packet_sender
                            .try_send(Arc::try_unwrap(video_packet).unwrap_or_default())
                        {
                            // If the channel is full, drop the packet
                            log::warn!("Video packet channel is full. Dropping packet.");
                        }

                        // Receive and process images
                        #[cfg(feature = "gst")]
                        if let Ok((image_data, pts)) = image_receiver.try_recv() {
                            // attach image to the stream_data.packet arc, clearing the current arc value
                            stream_data.packet = Arc::new(image_data.clone());
                            stream_data.has_image = image_data.len() as u8;
                            stream_data.packet_start = 0;
                            stream_data.packet_len = image_data.len();
                            stream_data.image_pts = pts;

                            // Process the received image data
                            debug!(
                                "Probe: Received a jpeg image with size: {} bytes",
                                image_data.len()
                            );
                        } else {
                            // zero out the packet data
                            stream_data.packet_start = 0;
                            stream_data.packet_len = 0;
                            stream_data.packet = Arc::new(Vec::new());
                        }
                    }
                } else {
                    // TODO:  Add SMPTE 2110 handling for line to frame conversion and other processing and analysis
                }

                // Watch file
                if args.watch_file != "" {
                    if let Ok(line) = watch_file_receiver.try_recv() {
                        info!("WatchFile Received line: {}", line);
                        // attach to stream_data.log_message
                        stream_data.log_message = line.clone();
                    }
                }

                if !args.extract_images && stream_data.packet_len > 0 {
                    // release the packet Arc so it can be reused
                    stream_data.packet = Arc::new(Vec::new()); // Create a new Arc<Vec<u8>> for the next packet
                    stream_data.packet_len = 0;
                    stream_data.packet_start = 0;
                }

                // Add the processed stream_data to the batch
                batch.push(stream_data);

                // Check if the batch size is reached or the batch timeout has elapsed
                if batch.len() >= args.kafka_batch_size
                    || last_batch_time.elapsed() >= batch_timeout
                {
                    // Send the batch to the Kafka thread
                    if ktx_clone1.send(batch).await.is_err() {
                        // If the channel is full, drop the batch and log a warning
                        log::warn!("Batch channel is full. Dropping batch.");
                    }
                    batch = Vec::new(); // Reset the batch
                    last_batch_time = tokio::time::Instant::now(); // Reset the batch timeout
                }
            }
        }
    });

    info!(
        "RsProbe: Starting up with Probe ID: {}",
        args.probe_id.clone()
    );

    let mut dot_last_sent_ts = Instant::now();
    let mut x_last_sent_ts = Instant::now();
    loop {
        match prx.try_recv() {
            Ok((packet, timestamp, iat)) => {
                if packet_count > 0 && packets_captured > packet_count {
                    println!(
                        "\nPacket count limit reached {}, signaling termination...",
                        packet_count
                    );
                    running.store(false, Ordering::SeqCst);
                    break;
                }
                packets_captured += 1;

                if !no_progress && dot_last_sent_ts.elapsed().as_secs() >= 1 {
                    dot_last_sent_ts = Instant::now();
                    print!(".");
                    // Flush stdout to ensure the progress dots are printed
                    io::stdout().flush().unwrap();
                }

                // Check if chunk is MPEG-TS or SMPTE 2110
                let chunk_type = is_mpegts_or_smpte2110(&packet[payload_offset..]);
                if chunk_type != 1 {
                    if chunk_type == 0 {
                        hexdump(&packet, 0, packet.len());
                        error!("Not MPEG-TS or SMPTE 2110");
                    }
                    is_mpegts = false;
                }

                let chunks = if is_mpegts {
                    process_mpegts_packet(
                        payload_offset,
                        packet,
                        packet_size,
                        start_time,
                        timestamp,
                        iat,
                        source_ip.clone(),
                        args.source_port,
                        args.probe_id.clone(),
                    )
                } else {
                    process_smpte2110_packet(
                        payload_offset,
                        packet,
                        packet_size,
                        start_time,
                        args.debug_smpte2110,
                        timestamp,
                        iat,
                        source_ip.clone(),
                        args.source_port,
                        args.probe_id.clone(),
                    )
                };

                // Send each chunk to the processing thread
                if chunk_sender.try_send(chunks).is_err() {
                    // If the channel is full, drop the chunk
                    log::warn!("Chunk channel is full. Dropping chunk.");
                }
            }
            Err(TryRecvError::Empty) => {
                // No packets received, print 'X' to indicate
                if !no_progress && x_last_sent_ts.elapsed().as_secs() >= 1 {
                    x_last_sent_ts = Instant::now();
                    print!("X");
                    // Flush stdout to ensure the progress dots are printed
                    io::stdout().flush().unwrap();
                }

                // Flush stdout to ensure the 'X' is printed
                io::stdout().flush().unwrap();
                // Sleep for a short duration to avoid high CPU usage
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
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

    // Stop the pipeline when done
    #[cfg(feature = "gst")]
    match pipeline.set_state(gst::State::Null) {
        Ok(_) => (),
        Err(err) => {
            eprintln!("Failed to set the pipeline state to Null: {}", err);
        }
    }

    println!("\nWaiting for threads to finish...");

    // Send Kafka stop signal
    let _ = ktx_clone2.try_send(Vec::new());
    drop(ktx_clone2);

    // Wait for the kafka thread to finish
    capture_task.await.unwrap();
    kafka_thread.await.unwrap();

    println!("\nThreads finished, exiting rsprobe");
}
