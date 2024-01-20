/*
 * rscap: probe.rs - Rust Stream Capture with pcap, output serialized stats to ZeroMQ
 *
 * Written in 2024 by Chris Kennedy (C) LTN Global
 *
 * License: MIT
 *
 */

use async_zmq;
use capnp;
use capnp::message::{Builder, HeapAllocator};
#[cfg(feature = "dpdk_enabled")]
use capsule::config::{load_config, DPDKConfig};
#[cfg(feature = "dpdk_enabled")]
use capsule::dpdk;
#[cfg(all(feature = "dpdk_enabled", target_os = "linux"))]
use capsule::prelude::*;
use clap::Parser;
use futures::stream::StreamExt;
use log::{debug, error, info};
use pcap::{Active, Capture, Device, PacketCodec};
use rscap::stream_data::{
    identify_video_pid, is_mpegts_or_smpte2110, parse_and_store_pat, process_packet,
    update_pid_map, Codec, PmtInfo, StreamData, Tr101290Errors, PAT_PID,
};
use rscap::{current_unix_timestamp_ms, hexdump};

use std::{
    error::Error as StdError,
    fmt,
    fs::File,
    io,
    io::Write,
    net::{IpAddr, Ipv4Addr, UdpSocket},
    sync::atomic::{AtomicBool, Ordering},
    sync::Arc,
    time::Instant,
};
use tokio::sync::mpsc::{self};
use zmq::PUSH;
// Include the generated paths for the Cap'n Proto schema
include!("../stream_data_capnp.rs");
// Video Processor Decoder
use h264_reader::annexb::AnnexBReader;
use h264_reader::nal::{pps, sei, slice, sps, Nal, RefNal, UnitType};
use h264_reader::push::NalInterest;
use h264_reader::Context;
//use rscap::videodecoder::VideoProcessor;
use rscap::stream_data::{process_mpegts_packet, process_smpte2110_packet};
use tokio::time::Duration;

// Define your custom PacketCodec
pub struct BoxCodec;

impl PacketCodec for BoxCodec {
    type Item = Box<[u8]>;

    fn decode(&mut self, packet: pcap::Packet) -> Self::Item {
        packet.data.into()
    }
}

fn is_cea_608(itu_t_t35_data: &sei::user_data_registered_itu_t_t35::ItuTT35) -> bool {
    // In this example, we check if the ITU-T T.35 data matches the known format for CEA-608.
    // This is a simplified example and might need adjustment based on the actual data format.
    match itu_t_t35_data {
        sei::user_data_registered_itu_t_t35::ItuTT35::UnitedStates => true,
        _ => false,
    }
}

// This function checks if the byte is a standard ASCII character
fn is_standard_ascii(byte: u8) -> bool {
    byte >= 0x20 && byte <= 0x7F
}

// Function to check if the byte pair represents XDS data
fn is_xds(byte1: u8, byte2: u8) -> bool {
    // Implement logic to identify XDS data
    // Placeholder logic: Example only
    byte1 == 0x01 && byte2 >= 0x20 && byte2 <= 0x7F
}

// Function to decode CEA-608 CC1/CC2
fn decode_cea_608_cc1_cc2(byte1: u8, byte2: u8) -> Option<String> {
    decode_character(byte1, byte2)
    // The above line replaces the previous implementation and uses decode_character
    // to handle both ASCII and control codes.
}

fn decode_cea_608_xds(byte1: u8, byte2: u8) -> Option<String> {
    if is_xds(byte1, byte2) {
        Some(format!("XDS: {:02X} {:02X}", byte1, byte2))
    } else {
        None
    }
}

// Decode CEA-608 characters, including control codes
fn decode_character(byte1: u8, byte2: u8) -> Option<String> {
    println!("Decoding: {:02X} {:02X}", byte1, byte2); // Debugging

    // Handle standard ASCII characters
    if is_standard_ascii(byte1) && is_standard_ascii(byte2) {
        return Some(format!("{}{}", byte1 as char, byte2 as char));
    }

    // Handle special control characters (Example)
    // This is a simplified version, actual implementation may vary based on control characters
    match (byte1, byte2) {
        (0x14, 0x2C) => Some(String::from("[Clear Caption]")),
        (0x14, 0x20) => Some(String::from("[Roll-Up Caption]")),
        // Add more control character handling here
        _ => {
            println!("Unhandled control character: {:02X} {:02X}", byte1, byte2); // Debugging
            None
        }
    }
}

// Simplified CEA-608 decoding function
// Main CEA-608 decoding function
fn decode_cea_608(data: &[u8]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut captions_cc1 = Vec::new();
    let mut captions_cc2 = Vec::new();
    let mut xds_data = Vec::new();

    for chunk in data.chunks(3) {
        if chunk.len() == 3 {
            match chunk[0] {
                0x04 => {
                    if let Some(decoded) = decode_cea_608_cc1_cc2(chunk[1], chunk[2]) {
                        captions_cc1.push(decoded);
                    } else if let Some(decoded) = decode_cea_608_xds(chunk[1], chunk[2]) {
                        xds_data.push(decoded);
                    }
                }
                0x05 => {
                    if let Some(decoded) = decode_cea_608_cc1_cc2(chunk[1], chunk[2]) {
                        captions_cc2.push(decoded);
                    }
                }
                _ => println!("Unknown caption channel: {:02X}", chunk[0]),
            }
        }
    }

    (captions_cc1, captions_cc2, xds_data)
}

// convert stream data sructure to capnp message
fn stream_data_to_capnp(stream_data: &StreamData) -> capnp::Result<Builder<HeapAllocator>> {
    let mut message = Builder::new_default();
    {
        let mut stream_data_msg = message.init_root::<stream_data_capnp::Builder>();

        stream_data_msg.set_pid(stream_data.pid);
        stream_data_msg.set_pmt_pid(stream_data.pmt_pid);
        stream_data_msg.set_program_number(stream_data.program_number);

        // Correct way to convert a String to ::capnp::text::Reader<'_>
        stream_data_msg.set_stream_type(stream_data.stream_type.as_str().into());

        stream_data_msg.set_continuity_counter(stream_data.continuity_counter);
        stream_data_msg.set_timestamp(stream_data.timestamp);
        stream_data_msg.set_bitrate(stream_data.bitrate);
        stream_data_msg.set_bitrate_max(stream_data.bitrate_max);
        stream_data_msg.set_bitrate_min(stream_data.bitrate_min);
        stream_data_msg.set_bitrate_avg(stream_data.bitrate_avg);
        stream_data_msg.set_iat(stream_data.iat);
        stream_data_msg.set_iat_max(stream_data.iat_max);
        stream_data_msg.set_iat_min(stream_data.iat_min);
        stream_data_msg.set_iat_avg(stream_data.iat_avg);
        stream_data_msg.set_error_count(stream_data.error_count);
        stream_data_msg.set_last_arrival_time(stream_data.last_arrival_time);
        stream_data_msg.set_start_time(stream_data.start_time);
        stream_data_msg.set_total_bits(stream_data.total_bits);
        stream_data_msg.set_count(stream_data.count);
        stream_data_msg.set_rtp_timestamp(stream_data.rtp_timestamp);
        stream_data_msg.set_rtp_payload_type(stream_data.rtp_payload_type);

        stream_data_msg
            .set_rtp_payload_type_name(stream_data.rtp_payload_type_name.as_str().into());

        stream_data_msg.set_rtp_line_number(stream_data.rtp_line_number);
        stream_data_msg.set_rtp_line_offset(stream_data.rtp_line_offset);
        stream_data_msg.set_rtp_line_length(stream_data.rtp_line_length);
        stream_data_msg.set_rtp_field_id(stream_data.rtp_field_id);
        stream_data_msg.set_rtp_line_continuation(stream_data.rtp_line_continuation);
        stream_data_msg.set_rtp_extended_sequence_number(stream_data.rtp_extended_sequence_number);
    }

    Ok(message)
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

/// RScap Probe Configuration
#[derive(Parser, Debug)]
#[clap(
    author = "Chris Kennedy",
    version = "1.2",
    about = "RsCap Probe for ZeroMQ output of MPEG-TS and SMPTE 2110 streams from pcap."
)]
struct Args {
    /// Sets the batch size
    #[clap(long, env = "BATCH_SIZE", default_value_t = 7)]
    batch_size: usize,

    /// Sets the payload offset
    #[clap(long, env = "PAYLOAD_OFFSET", default_value_t = 42)]
    payload_offset: usize,

    /// Sets the packet size
    #[clap(long, env = "PACKET_SIZE", default_value_t = 188)]
    packet_size: usize,

    /// Sets the read timeout
    #[clap(long, env = "READ_TIME_OUT", default_value_t = 60_000)]
    read_time_out: i32,

    /// Sets the target port
    #[clap(long, env = "TARGET_PORT", default_value_t = 5_556)]
    target_port: i32,

    /// Sets the target IP
    #[clap(long, env = "TARGET_IP", default_value = "127.0.0.1")]
    target_ip: String,

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

    /// Sets if Raw Stream should be sent
    #[clap(long, env = "SEND_RAW_STREAM", default_value_t = false)]
    send_raw_stream: bool,

    /// number of packets to capture
    #[clap(long, env = "PACKET_COUNT", default_value_t = 0)]
    packet_count: u64,

    /// Turn off progress output dots
    #[clap(long, env = "NO_PROGRESS", default_value_t = false)]
    no_progress: bool,

    /// Turn off ZeroMQ send
    #[clap(long, env = "NO_ZMQ", default_value_t = false)]
    no_zmq: bool,

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
    #[clap(long, env = "PCAP_CHANNEL_SIZE", default_value_t = 1_000)]
    pcap_channel_size: usize,

    /// MPSC Channel Size for PCAP
    #[clap(long, env = "ZMQ_CHANNEL_SIZE", default_value_t = 1_000)]
    zmq_channel_size: usize,

    /// MPSC Channel Size for Decoder
    #[clap(long, env = "DECODER_CHANNEL_SIZE", default_value_t = 1_000)]
    decoder_channel_size: usize,

    /// DPDK enable
    #[clap(long, env = "DPDK", default_value_t = false)]
    dpdk: bool,

    /// DPDK Port ID
    #[clap(long, env = "DPDK_PORT_ID", default_value_t = 0)]
    dpdk_port_id: u16,

    /// IPC Path for ZeroMQ
    #[clap(long, env = "IPC_PATH")]
    ipc_path: Option<String>,

    /// Output file for ZeroMQ
    #[clap(long, env = "OUTPUT_FILE", default_value = "")]
    output_file: String,

    /// Turn off the ZMQ thread
    #[clap(long, env = "NO_ZMQ_THREAD", default_value_t = false)]
    no_zmq_thread: bool,

    /// ZMQ Batch size
    #[clap(long, env = "ZMQ_BATCH_SIZE", default_value_t = 7)]
    zmq_batch_size: usize,

    /// Decode Video
    #[clap(long, env = "DECODE_VIDEO", default_value_t = false)]
    decode_video: bool,

    /// Decode Video Batch Size
    #[clap(long, env = "DECODE_VIDEO_BATCH_SIZE", default_value_t = 7)]
    decode_video_batch_size: usize,

    /// Debug SMPTE2110
    #[clap(long, env = "DEBUG_SMPTE2110", default_value_t = false)]
    debug_smpte2110: bool,

    /// Debug NALs
    #[clap(long, env = "DEBUG_NALS", default_value_t = false)]
    debug_nals: bool,

    /// List of NAL types to debug, comma separated: all, sps, pps, pic_timing, sei, slice, user_data_registered_itu_tt35, user_data_unregistered, buffering_period, unknown
    #[clap(
        long,
        env = "DEBUG_NAL_TYPES",
        default_value = "",
        help = "List of NAL types to debug, comma separated: all, sps, pps, pic_timing, sei, slice, user_data_registered_itu_tt35, user_data_unregistered, buffering_period, unknown"
    )]
    debug_nal_types: String,

    // Parse short NALs that are 0x000001
    #[clap(long, env = "PARSE_SHORT_NALS", default_value_t = false)]
    parse_short_nals: bool,
}

// MAIN Function
#[tokio::main]
async fn main() {
    let ctrl_c = tokio::signal::ctrl_c();

    tokio::select! {
        _ = ctrl_c => {
            println!("\nCtrl-C received, shutting down");
        }
        _ = rscap() => {
            println!("\nRsCap exited");
        }
    }
}

// RsCap Function
async fn rscap() {
    dotenv::dotenv().ok(); // read .env file

    let args = Args::parse();

    // Use the parsed arguments directly
    let mut batch_size = args.batch_size;
    let payload_offset = args.payload_offset;
    let mut packet_size = args.packet_size;
    let read_time_out = args.read_time_out;
    let target_port = args.target_port;
    let target_ip = args.target_ip;
    let source_device = args.source_device;
    let source_ip = args.source_ip;
    let source_protocol = args.source_protocol;
    let source_port = args.source_port;
    let debug_on = args.debug_on;
    let silent = args.silent;
    let use_wireless = args.use_wireless;
    let send_raw_stream = args.send_raw_stream;
    let packet_count = args.packet_count;
    let no_progress = args.no_progress;
    let no_zmq = args.no_zmq;
    let promiscuous = args.promiscuous;
    let show_tr101290 = args.show_tr101290;
    let mut buffer_size = args.buffer_size as i64;
    let mut immediate_mode = args.immediate_mode;
    let pcap_stats = args.pcap_stats;
    let mut pcap_channel_size = args.pcap_channel_size;
    let mut zmq_channel_size = args.zmq_channel_size;
    let mut decoder_channel_size = args.decoder_channel_size;
    #[cfg(all(feature = "dpdk_enabled", target_os = "linux"))]
    let use_dpdk = args.dpdk;
    let output_file = args.output_file;
    let no_zmq_thread = args.no_zmq_thread;
    let mut zmq_batch_size = args.zmq_batch_size;
    let debug_smpte2110 = args.debug_smpte2110;
    let decode_video = args.decode_video;
    let mut decode_video_batch_size = args.decode_video_batch_size;
    let debug_nals = args.debug_nals;
    let debug_nal_types = args.debug_nal_types;
    let parse_short_nals = args.parse_short_nals;

    println!("Starting RsCap Probe...");

    // turn debug_nal_types into a vector
    // Assuming debug_nal_types is a String
    let debug_nal_types: Vec<String> = debug_nal_types.split(',').map(|s| s.to_string()).collect();

    // SMPTE2110 specific settings
    if args.smpte2110 {
        immediate_mode = true; // set immediate mode to true for smpte2110
        buffer_size = 10_000_000_000; // set pcap buffer size to 10GB for smpte2110
        pcap_channel_size = 1_000_000; // set pcap channel size for smpte2110
        zmq_channel_size = 1_000_000; // set zmq channel size for smpte2110
        decoder_channel_size = 1_000_000; // set decoder channel size for smpte2110
        packet_size = 1_220; // set packet size to 1220 (body) + 12 (header) for RTP
        batch_size = 3; // N x 1220 size packets for pcap read size
        decode_video_batch_size = 7; // N x 1220 size packets for pcap read size
        zmq_batch_size = 1080; // N x packets for how many packets to send to ZMQ per batch
    }

    if silent {
        // set log level to error
        std::env::set_var("RUST_LOG", "error");
    }

    // calculate read size based on batch size and packet size
    let read_size: i32 = (packet_size as i32 * batch_size as i32) + payload_offset as i32; // pcap read size

    let mut is_mpegts = true; // Default to true, update based on actual packet type

    // Initialize logging
    let _ = env_logger::try_init();

    let (ptx, mut prx) = mpsc::channel::<Arc<Vec<u8>>>(pcap_channel_size);

    let running = Arc::new(AtomicBool::new(true));
    let running_capture = running.clone();
    let running_decoder = running.clone();
    let running_zmq = running.clone();

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
            while running_capture.load(Ordering::SeqCst) {
                match port.rx_burst(&mut packets) {
                    Ok(_) => {
                        for packet in packets.drain(..) {
                            // Extract data from the packet
                            let data = packet.data();

                            // Convert to Arc<Vec<u8>> to maintain consistency with pcap logic
                            let packet_data = Arc::new(data.to_vec());

                            // Send packet data to processing channel
                            ptx.send(packet_data).await.unwrap();

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
                &source_ip,
            )
            .expect("Failed to initialize pcap");

            // Create a PacketStream from the Capture
            let mut stream = cap.stream(BoxCodec).unwrap();
            let mut count = 0;

            let mut stats_last_sent_ts = Instant::now();
            let mut packets_dropped = 0;

            while running_capture.load(Ordering::SeqCst) {
                while let Some(packet) = stream.next().await {
                    match packet {
                        Ok(data) => {
                            count += 1;
                            let packet_data = Arc::new(data.to_vec());
                            ptx.send(packet_data).await.unwrap();
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
                                    "#{} Current stats: Received: {}, Dropped: {}/{}, Interface Dropped: {} packet_size: {} bytes.",
                                    count, stats.received, stats.dropped - packets_dropped, stats.dropped, stats.if_dropped, data.len(),
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

    let mut ctx = Context::default();
    let mut scratch = Vec::new();
    // Use the `move` keyword to move ownership of `ctx` and `scratch` into the closure
    let mut annexb_reader = AnnexBReader::accumulate(move |nal: RefNal<'_>| {
        if !nal.is_complete() {
            return NalInterest::Buffer;
        }
        let hdr = match nal.header() {
            Ok(h) => h,
            Err(e) => {
                // check if we are in debug mode for nals, else check if this is a ForbiddenZeroBit error, which we ignore
                let e_str = format!("{:?}", e);
                if !debug_nals && e_str == "ForbiddenZeroBit" {
                    // ignore forbidden zero bit error unless we are in debug mode
                } else {
                    // show nal contents
                    debug!("---\n{:?}\n---", nal);
                    error!("Failed to parse NAL header: {:?}", e);
                }
                return NalInterest::Buffer;
            }
        };
        match hdr.nal_unit_type() {
            UnitType::SeqParameterSet => {
                if let Ok(sps) = sps::SeqParameterSet::from_bits(nal.rbsp_bits()) {
                    // check if debug_nal_types has sps
                    if debug_nal_types.contains(&"sps".to_string())
                        || debug_nal_types.contains(&"all".to_string())
                    {
                        println!("Found SPS: {:?}", sps);
                    }
                    ctx.put_seq_param_set(sps);
                }
            }
            UnitType::PicParameterSet => {
                if let Ok(pps) = pps::PicParameterSet::from_bits(&ctx, nal.rbsp_bits()) {
                    // check if debug_nal_types has pps
                    if debug_nal_types.contains(&"pps".to_string())
                        || debug_nal_types.contains(&"all".to_string())
                    {
                        println!("Found PPS: {:?}", pps);
                    }
                    ctx.put_pic_param_set(pps);
                }
            }
            UnitType::SEI => {
                let mut r = sei::SeiReader::from_rbsp_bytes(nal.rbsp_bytes(), &mut scratch);
                while let Ok(Some(msg)) = r.next() {
                    match msg.payload_type {
                        sei::HeaderType::PicTiming => {
                            let sps = match ctx.sps().next() {
                                Some(s) => s,
                                None => continue,
                            };
                            let pic_timing = sei::pic_timing::PicTiming::read(sps, &msg);
                            match pic_timing {
                                Ok(pic_timing_data) => {
                                    // Check if debug_nal_types has pic_timing or all
                                    if debug_nal_types.contains(&"pic_timing".to_string())
                                        || debug_nal_types.contains(&"all".to_string())
                                    {
                                        println!("Found PicTiming: {:?}", pic_timing_data);
                                    }
                                }
                                Err(e) => {
                                    error!("Error parsing PicTiming SEI: {:?}", e);
                                }
                            }
                        }
                        h264_reader::nal::sei::HeaderType::BufferingPeriod => {
                            let sps = match ctx.sps().next() {
                                Some(s) => s,
                                None => continue,
                            };
                            let buffering_period =
                                sei::buffering_period::BufferingPeriod::read(&ctx, &msg);
                            // check if debug_nal_types has buffering_period
                            if debug_nal_types.contains(&"buffering_period".to_string())
                                || debug_nal_types.contains(&"all".to_string())
                            {
                                println!(
                                    "Found BufferingPeriod: {:?} Payload: [{:?}] - {:?}",
                                    buffering_period, msg.payload, sps
                                );
                            }
                        }
                        h264_reader::nal::sei::HeaderType::UserDataRegisteredItuTT35 => {
                            match sei::user_data_registered_itu_t_t35::ItuTT35::read(&msg) {
                                Ok((itu_t_t35_data, remaining_data)) => {
                                    if debug_nal_types
                                        .contains(&"user_data_registered_itu_tt35".to_string())
                                        || debug_nal_types.contains(&"all".to_string())
                                    {
                                        println!("Found UserDataRegisteredItuTT35: {:?}, Remaining Data: {:?}", itu_t_t35_data, remaining_data);
                                    }
                                    if is_cea_608(&itu_t_t35_data) {
                                        let (captions_cc1, captions_cc2, xds_data) =
                                            decode_cea_608(remaining_data);
                                        if !captions_cc1.is_empty() {
                                            println!("CEA-608 CC1 Captions: {:?}", captions_cc1);
                                        }
                                        if !captions_cc2.is_empty() {
                                            println!("CEA-608 CC2 Captions: {:?}", captions_cc2);
                                        }
                                        if !xds_data.is_empty() {
                                            println!("CEA-608 XDS Data: {:?}", xds_data);
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!("Error parsing ITU T.35 data: {:?}", e);
                                }
                            }
                        }
                        h264_reader::nal::sei::HeaderType::UserDataUnregistered => {
                            // Check if debug_nal_types has user_data_unregistered or all
                            if debug_nal_types.contains(&"user_data_unregistered".to_string())
                                || debug_nal_types.contains(&"all".to_string())
                            {
                                println!(
                                    "Found SEI type UserDataUnregistered {:?} payload: [{:?}]",
                                    msg.payload_type, msg.payload
                                );
                            }
                        }
                        _ => {
                            // check if debug_nal_types has sei
                            if debug_nal_types.contains(&"sei".to_string())
                                || debug_nal_types.contains(&"all".to_string())
                            {
                                println!(
                                    "Unknown Found SEI type {:?} payload: [{:?}]",
                                    msg.payload_type, msg.payload
                                );
                            }
                        }
                    }
                }
            }
            UnitType::SliceLayerWithoutPartitioningIdr
            | UnitType::SliceLayerWithoutPartitioningNonIdr => {
                let msg = slice::SliceHeader::from_bits(&ctx, &mut nal.rbsp_bits(), hdr);
                // check if debug_nal_types has slice
                if debug_nal_types.contains(&"slice".to_string()) {
                    println!("Found NAL Slice: {:?}", msg);
                }
            }
            _ => {
                // check if debug_nal_types has nal
                if debug_nal_types.contains(&"unknown".to_string()) {
                    println!("Found Unknown NAL: {:?}", nal);
                }
            }
        }
        NalInterest::Buffer
    });

    // Initialize the video processor
    // Setup channel for passing data between threads
    let (dtx, mut drx) = mpsc::channel::<Vec<StreamData>>(decoder_channel_size);

    // Spawn a new thread for Decoder communication
    let decoder_thread = tokio::spawn(async move {
        loop {
            if !running_decoder.load(Ordering::SeqCst) {
                debug!("Decoder thread received stop signal.");
                break;
            }

            if !decode_video {
                // Sleep for a short duration to prevent a tight loop
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }

            // Use tokio::select to simultaneously wait for a new batch or a stop signal
            tokio::select! {
                Some(mut batch) = drx.recv() => {
                    debug!("Processing {} video packets in decoder thread", batch.len());
                    for stream_data in &batch {
                        // packet is a subset of the original packet, starting at the payload
                        let packet_start = stream_data.packet_start;
                        let packet_end = stream_data.packet_start + stream_data.packet_len;

                        if packet_end - packet_start > packet_size {
                            error!("NAL Parser: Packet size {} is larger than packet buffer size {}. Skipping packet.",
                                packet_end - packet_start, packet_size);
                            continue;
                        }

                        // check if packet_start + 4 is less than packet_end
                        if packet_start + 4 >= packet_end {
                            continue;
                        }

                        // Skip MPEG-TS header and adaptation field
                        let header_len = 4;
                        let adaptation_field_control = (stream_data.packet[packet_start + 3] & 0b00110000) >> 4;

                        if adaptation_field_control == 0b10 {
                            continue; // Skip packets with only adaptation field (no payload)
                        }

                        let payload_start = if adaptation_field_control != 0b01 {
                            header_len + 1 + stream_data.packet[packet_start + 4] as usize
                        } else {
                            header_len
                        };

                        // confirm payload_start is sane
                        if payload_start >= packet_end || packet_end - payload_start < 4 {
                            error!("NAL Parser: Payload start {} is invalid with packet_start as {} and packet_end as {}. Skipping packet.",
                                payload_start, packet_start, packet_end);
                            continue;
                        }

                        // Process payload, skipping padding bytes
                        let mut pos = payload_start;
                        while pos + 4 < packet_end {
                            if parse_short_nals && stream_data.packet[pos..pos + 3] == [0x00, 0x00, 0x01] {
                                let nal_start = pos;
                                pos += 3; // Move past the short start code

                                // Search for the next start code
                                while pos + 4 <= packet_end &&
                                      stream_data.packet[pos..pos + 4] != [0x00, 0x00, 0x00, 0x01] {
                                    // Check for short start code, 0xff padding, or 0x00000000 sequence
                                    if stream_data.packet[pos..pos + 3] == [0x00, 0x00, 0x01] && pos > nal_start + 3 {
                                        // Found a short start code, so back up and process the NAL unit
                                        break;
                                    } else if stream_data.packet[pos + 1] == 0xff && pos > nal_start + 3 {
                                        // check for 0xff padding and that we are at least 2 bytes into the nal
                                        break;
                                    } else if stream_data.packet[pos..pos + 3] == [0x00, 0x00, 0x00] && pos > nal_start + 3 {
                                        // check for 0x00 0x00 0x00 0x00 sequence to stop at
                                        break;
                                    }
                                    pos += 1;
                                }

                                // check if we only have 4 bytes left in the packet, if so then collect them too
                                if pos + 4 >= packet_end {
                                    while pos < packet_end {
                                        if stream_data.packet[pos..pos + 1] == [0xff] {
                                            // check for 0xff padding and that we are at least 2 bytes into the nal
                                            break;
                                        } else if pos + 2 < packet_end && stream_data.packet[pos..pos + 2] == [0x00, 0x00] {
                                            // check for 0x00 0x00 sequence to stop at
                                            break;
                                        }
                                        pos += 1;
                                    }
                                }

                                let nal_end = pos; // End of NAL unit found or end of packet
                                if nal_end - nal_start > 3 { // Threshold for significant NAL unit size
                                    let nal_unit = &stream_data.packet[nal_start..nal_end];

                                    // Debug print the NAL unit
                                    if debug_nals {
                                        let packet_len = nal_end - nal_start;
                                        info!("Extracted {} byte Short NAL Unit from packet range {}-{}:", packet_len, nal_start, nal_end);
                                        let nal_unit_arc = Arc::new(nal_unit.to_vec());
                                        hexdump(&nal_unit_arc, 0, packet_len);
                                    }

                                    // Process the NAL unit
                                    annexb_reader.push(nal_unit);
                                    annexb_reader.reset();
                                }
                            } else if pos + 4 < packet_end && stream_data.packet[pos..pos + 4] == [0x00, 0x00, 0x00, 0x01] {
                                let nal_start = pos;
                                pos += 4; // Move past the long start code

                                // Search for the next start code
                                while pos + 4 <= packet_end &&
                                      stream_data.packet[pos..pos + 4] != [0x00, 0x00, 0x00, 0x01] {
                                    // Check for short start code
                                    if stream_data.packet[pos..pos + 3] == [0x00, 0x00, 0x01] && pos > nal_start + 3 {
                                        // Found a short start code, so back up and process the NAL unit
                                        break;
                                    } else if stream_data.packet[pos + 1] == 0xff && pos > nal_start + 3 {
                                        // check for 0xff padding and that we are at least 2 bytes into the nal
                                        break;
                                    } else if stream_data.packet[pos..pos + 3] == [0x00, 0x00, 0x00] && pos > nal_start + 3 {
                                        // check for 0x00 0x00 0x00 0x00 sequence to stop at
                                        break;
                                    }
                                    pos += 1;
                                }

                                // check if we only have 4 bytes left in the packet, if so then collect them too
                                if pos + 4 >= packet_end {
                                    while pos < packet_end {
                                        if stream_data.packet[pos..pos + 1] == [0xff] {
                                            // check for 0xff padding and that we are at least 2 bytes into the nal
                                            break;
                                        } else if pos + 2 < packet_end && stream_data.packet[pos..pos + 2] == [0x00, 0x00] {
                                            // check for 0x00 0x00 sequence to stop at
                                            break;
                                        }
                                        pos += 1;
                                    }
                                }

                                let nal_end = pos; // End of NAL unit found or end of packet
                                if nal_end - nal_start > 3 { // Threshold for significant NAL unit size
                                    let nal_unit = &stream_data.packet[nal_start..nal_end];

                                    // Debug print the NAL unit
                                    if debug_nals {
                                        let packet_len = nal_end - nal_start;
                                        let nal_unit_arc = Arc::new(nal_unit.to_vec());
                                        hexdump(&nal_unit_arc, 0, packet_len);
                                        info!("Extracted {} byte Long NAL Unit from packet range {}-{}:", packet_len, nal_start, nal_end);
                                    }

                                    // Process the NAL unit
                                    annexb_reader.push(nal_unit);
                                    annexb_reader.reset();
                                }
                            } else {
                                pos += 1; // Move to the next byte if no start code found
                            }
                        }
                    }
                    // Clear the batch after processing
                    batch.clear();
                }
                _ = tokio::time::sleep(Duration::from_millis(10)), if !running_decoder.load(Ordering::SeqCst) => {
                    // This branch allows checking the running flag regularly
                    info!("Decoder thread received stop signal.");
                    break;
                }
            }
        }
    });

    // Setup channel for passing stream_data for ZMQ thread sending the stream data to monitor process
    let (tx, mut rx) = mpsc::channel::<Vec<StreamData>>(zmq_channel_size);

    // Spawn a new thread for ZeroMQ communication
    let zmq_thread = tokio::spawn(async move {
        let context = async_zmq::Context::new();
        let publisher = context.socket(PUSH).unwrap();
        // Determine the connection endpoint (IPC if provided, otherwise TCP)
        let endpoint = if let Some(ipc_path_copy) = args.ipc_path {
            format!("ipc://{}", ipc_path_copy)
        } else {
            format!("tcp://{}:{}", target_ip, target_port)
        };
        info!("ZeroMQ publisher startup {}", endpoint);

        publisher.bind(&endpoint).unwrap();

        // Initialize an Option<File> to None
        let mut file = if !output_file.is_empty() {
            Some(File::create(&output_file).unwrap())
        } else {
            None
        };

        let mut dot_last_sent_ts = Instant::now();

        while running_zmq.load(Ordering::SeqCst) {
            while let Some(mut batch) = rx.recv().await {
                // Process and send messages
                for stream_data in batch.iter() {
                    // Serialize StreamData to Cap'n Proto message
                    let capnp_message = stream_data_to_capnp(stream_data)
                        .expect("Failed to convert to Cap'n Proto message");
                    let mut serialized_data = Vec::new();
                    capnp::serialize::write_message(&mut serialized_data, &capnp_message)
                        .expect("Failed to serialize Cap'n Proto message");

                    // Create ZeroMQ message from serialized Cap'n Proto data
                    let capnp_msg = zmq::Message::from(serialized_data);

                    // Send the Cap'n Proto message
                    if !no_zmq {
                        publisher.send(capnp_msg, zmq::SNDMORE).unwrap();
                    }

                    let packet_slice = &stream_data.packet[stream_data.packet_start
                        ..stream_data.packet_start + stream_data.packet_len];
                    let packet_msg = if send_raw_stream {
                        // Write to file if output_file is provided
                        if let Some(file) = file.as_mut() {
                            if !no_progress && dot_last_sent_ts.elapsed().as_secs() >= 1 {
                                dot_last_sent_ts = Instant::now();
                                print!("*");
                                // Flush stdout to ensure the progress dots are printed
                                io::stdout().flush().unwrap();
                            }
                            file.write_all(&packet_slice).unwrap();
                        }
                        zmq::Message::from(packet_slice)
                    } else {
                        zmq::Message::from(Vec::new())
                    };

                    // Send the raw packet
                    if !no_zmq {
                        publisher.send(packet_msg, 0).unwrap();
                    }
                }
                batch.clear();
            }
        }
    });

    // Perform TR 101 290 checks
    let mut tr101290_errors = Tr101290Errors::new();

    // start time
    let start_time = current_unix_timestamp_ms().unwrap_or(0);

    let mut packets_captured = 0;

    // Start packet capture
    let mut batch = Vec::new();
    let mut video_batch = Vec::new();
    let mut video_pid: Option<u16> = Some(0xFFFF);
    let mut video_codec: Option<Codec> = Some(Codec::NONE);
    let mut current_video_frame = Vec::<StreamData>::new();
    let mut pmt_info: PmtInfo = PmtInfo {
        pid: 0xFFFF,
        packet: Vec::new(),
    };

    let mut dot_last_sent_ts = Instant::now();
    while let Some(packet) = prx.recv().await {
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
            process_mpegts_packet(payload_offset, packet, packet_size, start_time)
        } else {
            process_smpte2110_packet(
                payload_offset,
                packet,
                packet_size,
                start_time,
                debug_smpte2110,
            )
        };

        // Process each chunk
        for mut stream_data in chunks {
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

            let mut pid: u16 = 0xFFFF;

            if is_mpegts {
                pid = stream_data.pid;
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
                            update_pid_map(&packet_chunk, &pmt_info.packet);
                            // Identify the video PID (if not already identified)
                            if let Some((new_pid, new_codec)) = identify_video_pid(&packet_chunk) {
                                if video_pid.map_or(true, |vp| vp != new_pid) {
                                    video_pid = Some(new_pid);
                                    info!(
                                        "STATUS::VIDEO_PID:CHANGE: to {}/{} from {}/{}",
                                        new_pid,
                                        new_codec.clone(),
                                        video_pid.unwrap(),
                                        video_codec.unwrap()
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

            // Check for TR 101 290 errors
            process_packet(
                &mut stream_data,
                &mut tr101290_errors,
                is_mpegts,
                pmt_info.pid,
            );

            // If MpegTS, Check if this is a video PID and if so parse NALS and decode video
            if is_mpegts {
                if pid == video_pid.unwrap_or(0xFFFF) {
                    if decode_video && video_batch.len() >= decode_video_batch_size {
                        dtx.send(video_batch).await.unwrap(); // Clone if necessary
                        video_batch = Vec::new();
                    } else if decode_video {
                        let mut stream_data_clone = stream_data.clone();
                        stream_data_clone.packet_start = stream_data.packet_start;
                        stream_data_clone.packet_len = stream_data.packet_len;
                        stream_data_clone.packet = Arc::new(stream_data.packet.to_vec());
                        video_batch.push(stream_data_clone);
                    }
                } else {
                    // TODO: Add handling for other PIDs besides video only
                }
            } else {
                // TODO:  Add SMPTE 2110 handling for line to frame conversion and other processing and analysis
            }

            // release the packet Arc so it can be reused
            if !send_raw_stream && stream_data.packet_len > 0 {
                stream_data.packet = Arc::new(Vec::new()); // Create a new Arc<Vec<u8>> for the next packet
                stream_data.packet_len = 0;
                stream_data.packet_start = 0;
                if pid == 0x1FFF && is_mpegts {
                    continue;
                }
            } else if send_raw_stream {
                // Skip null packets
                if pid == 0x1FFF && is_mpegts {
                    // clear the Arc so it can be reused
                    stream_data.packet = Arc::new(Vec::new()); // Create a new Arc<Vec<u8>> for the next packet
                    continue;
                }
            }
            batch.push(stream_data);

            //info!("STATUS::PACKETS:CAPTURED: {}", packets_captured);
            // Check if batch is full
            if !no_zmq_thread {
                if batch.len() >= zmq_batch_size {
                    //info!("STATUS::BATCH:SEND: {}", batch.len());
                    // Send the batch to the channel
                    tx.send(batch).await.unwrap();
                    // release the packet Arc so it can be reused
                    batch = Vec::new(); // Create a new Vec for the next batch
                } else {
                    //info!("STATUS::BATCH:WAIT: {}", batch.len());
                }
            } else {
                // go through each stream_data and release the packet Arc so it can be reused
                for stream_data in batch.iter_mut() {
                    stream_data.packet = Arc::new(Vec::new()); // Create a new Arc<Vec<u8>> for the next packet
                }
                // disgard the batch, we don't send it anywhere
                batch.clear();
                batch = Vec::new(); // Create a new Vec for the next batch
                                    //info!("STATUS::BATCH:DISCARD: {}", batch.len());
            }
        }
    }

    println!("\nSending stop signals to threads...");

    // Send ZMQ stop signal
    tx.send(Vec::new()).await.unwrap();
    drop(tx);

    // Send Decoder stop signal
    dtx.send(Vec::new()).await.unwrap();
    drop(dtx);

    println!("\nWaiting for threads to finish...");

    // Wait for the zmq_thread to finish
    capture_task.await.unwrap();
    zmq_thread.await.unwrap();
    decoder_thread.await.unwrap();

    println!("\nThreads finished, exiting rscap probe");
}
