/*
 * rscap: probe.rs - Rust Stream Capture with pcap, output serialized stats to ZeroMQ
 *
 * Written in 2024 by Chris Kennedy (C)
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
#[cfg(feature = "gst")]
use gstreamer as gst;
#[cfg(feature = "gst")]
use gstreamer::prelude::*;
use log::{debug, error, info, warn};
use pcap::{Active, Capture, Device, PacketCodec};
use rscap::stream_data::{
    identify_video_pid, is_mpegts_or_smpte2110, parse_and_store_pat, process_packet,
    update_pid_map, Codec, PmtInfo, StreamData, Tr101290Errors, PAT_PID,
};
#[cfg(feature = "gst")]
use rscap::stream_data::{initialize_pipeline, process_video_packets, pull_images};
use rscap::system_stats::get_system_stats;
use rscap::watch_file::watch_daemon;
use rscap::{current_unix_timestamp_ms, hexdump};
use std::sync::mpsc::channel;
//#[cfg(feature = "gst")]
//use std::sync::Mutex;
use std::thread;
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
//use rscap::videodecoder::VideoProcessor;
use env_logger::{Builder as LogBuilder, Env};
use rscap::stream_data::{process_mpegts_packet, process_smpte2110_packet};

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
        stream_data_msg.set_stream_type_number(stream_data.stream_type_number);

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
        stream_data_msg.set_current_error_count(stream_data.current_error_count);
        stream_data_msg.set_last_arrival_time(stream_data.last_arrival_time);
        stream_data_msg.set_capture_time(stream_data.capture_time);
        stream_data_msg.set_capture_iat(stream_data.capture_iat);
        stream_data_msg.set_last_sample_time(stream_data.last_sample_time);
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
        stream_data_msg.set_source_ip(stream_data.source_ip.as_str().into());
        stream_data_msg.set_source_port(stream_data.source_port as u32);

        // System stats fields
        stream_data_msg.set_total_memory(stream_data.total_memory);
        stream_data_msg.set_used_memory(stream_data.used_memory);
        stream_data_msg.set_total_swap(stream_data.total_swap);
        stream_data_msg.set_used_swap(stream_data.used_swap);
        stream_data_msg.set_cpu_usage(stream_data.cpu_usage);
        stream_data_msg.set_cpu_count(stream_data.cpu_count as u32);
        stream_data_msg.set_core_count(stream_data.core_count as u32);
        stream_data_msg.set_boot_time(stream_data.boot_time);
        stream_data_msg.set_load_avg_one(stream_data.load_avg_one);
        stream_data_msg.set_load_avg_five(stream_data.load_avg_five);
        stream_data_msg.set_load_avg_fifteen(stream_data.load_avg_fifteen);
        stream_data_msg.set_host_name(stream_data.host_name.as_str().into());
        stream_data_msg.set_kernel_version(stream_data.kernel_version.as_str().into());
        stream_data_msg.set_os_version(stream_data.os_version.as_str().into());
        stream_data_msg.set_has_image(stream_data.has_image);
        stream_data_msg.set_image_pts(stream_data.image_pts);
        stream_data_msg.set_capture_iat_max(stream_data.capture_iat_max);
        stream_data_msg.set_log_message(stream_data.log_message.as_str().into());
        stream_data_msg.set_probe_id(stream_data.probe_id.as_str().into());
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
    version = "0.5.30",
    about = "RsCap Probe for ZeroMQ output of MPEG-TS and SMPTE 2110 streams from pcap."
)]
struct Args {
    /// probe ID - ID for the probe to send with the messages
    #[clap(long, env = "PROBE_ID", default_value = "")]
    probe_id: String,

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
    #[clap(long, env = "READ_TIME_OUT", default_value_t = 300_000)]
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

    /// Send Null Packets
    #[clap(long, env = "SEND_NULL_PACKETS", default_value_t = false)]
    send_null_packets: bool,

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
    #[clap(long, env = "PCAP_CHANNEL_SIZE", default_value_t = 10_000)]
    pcap_channel_size: usize,

    /// MPSC Channel Size for PCAP
    #[clap(long, env = "ZMQ_CHANNEL_SIZE", default_value_t = 100_000)]
    zmq_channel_size: usize,

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
    #[clap(long, env = "ZMQ_BATCH_SIZE", default_value_t = 1000)]
    zmq_batch_size: usize,

    /// Debug SMPTE2110
    #[clap(long, env = "DEBUG_SMPTE2110", default_value_t = false)]
    debug_smpte2110: bool,

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

    /// Image Height - Image height in pixels of Thumbnail extracted images
    #[clap(long, env = "IMAGE_HEIGHT", default_value_t = 240)]
    image_height: u32,

    /// filmstrip length
    #[clap(long, env = "FILMSTRIP_LENGTH", default_value_t = 10)]
    filmstrip_length: usize,

    /// Watch File - File we watch for changes to send as the streams.PID.log_line string
    #[clap(long, env = "WATCH_FILE", default_value = "")]
    watch_file: String,

    /// Gstreamer Queue Buffers
    #[clap(long, env = "GST_QUEUE_BUFFERS", default_value_t = 1)]
    gst_queue_buffers: u32,

    /// Scale Images - Scale the images to the specified height
    #[clap(long, env = "SCALE_IMAGES", default_value_t = false)]
    scale_images: bool,

    /// Jpeg Quality - Quality of the Jpeg images
    #[clap(long, env = "JPEG_QUALITY", default_value_t = 70)]
    jpeg_quality: u8,

    /// Input Codec - Expected codec type for Video stream, limited to h264, h265 or mpeg2.
    #[clap(long, env = "INPUT_CODEC", default_value = "h264")]
    input_codec: String,

    /// image framerate - Framerate of the images extracted in 1/1 format
    #[clap(long, env = "IMAGE_FRAMERATE", default_value = "1/10")]
    image_framerate: String,

    /// Loglevel - Log level for the application
    #[clap(long, env = "LOGLEVEL", default_value = "info")]
    loglevel: String,
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
        _ = rscap(running.clone()) => {
            println!("\nRsCap exited");
        }
    }
}

// RsCap Function
async fn rscap(running: Arc<AtomicBool>) {
    let running_capture = running.clone();
    let running_zmq = running.clone();
    let running_gstreamer_process = running.clone();
    let running_gstreamer_pull = running.clone();

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
    let source_ip = args.source_ip.clone();
    let source_protocol = args.source_protocol;
    let source_port = args.source_port;
    let debug_on = args.debug_on;
    let silent = args.silent;
    let use_wireless = args.use_wireless;
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
    #[cfg(all(feature = "dpdk_enabled", target_os = "linux"))]
    let use_dpdk = args.dpdk;
    let mut zmq_batch_size = args.zmq_batch_size;

    println!("Starting RsCap Probe...");

    // SMPTE2110 specific settings
    if args.smpte2110 {
        immediate_mode = true; // set immediate mode to true for smpte2110
        buffer_size = 10_000_000_000; // set pcap buffer size to 10GB for smpte2110
        pcap_channel_size = 1_000_000; // set pcap channel size for smpte2110
        zmq_channel_size = 1_000_000; // set zmq channel size for smpte2110
        packet_size = 1_220; // set packet size to 1220 (body) + 12 (header) for RTP
        batch_size = 3; // N x 1220 size packets for pcap read size
        zmq_batch_size = 1080; // N x packets for how many packets to send to ZMQ per batch
    }

    if silent {
        // set log level to error
        std::env::set_var("RUST_LOG", "error");
    }

    // calculate read size based on batch size and packet size
    let read_size: i32 = (packet_size as i32 * batch_size as i32) + payload_offset as i32; // pcap read size

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
                &source_ip,
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

                            ptx.send((packet_data, timestamp_ms, iat)).await.unwrap();

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
        let mut file = if !args.output_file.is_empty() {
            Some(File::create(&args.output_file).unwrap())
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
                    let packet_msg = if stream_data.packet_len > 0
                        && (args.send_raw_stream || args.extract_images)
                    {
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

    // Create channels for sending video packets and receiving images
    #[cfg(feature = "gst")]
    let (video_packet_sender, video_packet_receiver) = mpsc::channel(10000);
    #[cfg(feature = "gst")]
    let (image_sender, mut image_receiver) = mpsc::channel(10000);

    // Initialize the pipeline
    #[cfg(feature = "gst")]
    let (pipeline, appsrc, appsink) = match initialize_pipeline(
        &args.input_codec,
        args.image_height,
        args.gst_queue_buffers,
        args.scale_images,
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
        running_gstreamer_pull,
    );

    // Watch file thread and sender/receiver for log file input
    let (watch_file_sender, watch_file_receiver) = channel();
    let watch_file_sender_clone = watch_file_sender.clone();

    if args.watch_file != "" {
        let watch_file_clone = args.watch_file.clone();
        thread::spawn(move || {
            watch_daemon(&watch_file_clone, watch_file_sender_clone);
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
    let mut batch = Vec::new();
    let mut video_pid: Option<u16> = Some(0xFFFF);
    let mut video_codec: Option<Codec> = Some(Codec::NONE);
    let mut current_video_frame = Vec::<StreamData>::new();
    let mut pmt_info: PmtInfo = PmtInfo {
        pid: 0xFFFF,
        packet: Vec::new(),
    };

    // OS and Network stats
    let system_stats = get_system_stats();

    let mut video_stream_type = 0;

    info!("RsCap: Starting up with Probe ID: {}", args.probe_id);

    info!("Startup System OS Stats:\n{:?}", system_stats);

    let mut dot_last_sent_ts = Instant::now();
    while let Some((packet, timestamp, iat)) = prx.recv().await {
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
                args.source_ip.clone(),
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
                args.source_ip.clone(),
                args.source_port,
                args.probe_id.clone(),
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
                            update_pid_map(
                                &packet_chunk,
                                &pmt_info.packet,
                                timestamp,
                                iat,
                                args.source_ip.clone(),
                                args.source_port,
                                args.probe_id.clone(),
                            );
                            // Identify the video PID (if not already identified)
                            if let Some((new_pid, new_codec)) = identify_video_pid(&packet_chunk) {
                                if video_pid.map_or(true, |vp| vp != new_pid) {
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
                args.probe_id.clone(),
            );

            // If MpegTS, Check if this is a video PID and if so parse NALS and decode video
            if is_mpegts {
                // Process video packets
                #[cfg(feature = "gst")]
                if args.extract_images {
                    #[cfg(feature = "gst")]
                    if video_stream_type > 0 {
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
                            warn!("Video packet channel is full. Dropping packet.");
                        }
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

            if !args.extract_images && !args.send_raw_stream && stream_data.packet_len > 0 {
                // release the packet Arc so it can be reused
                stream_data.packet = Arc::new(Vec::new()); // Create a new Arc<Vec<u8>> for the next packet
                stream_data.packet_len = 0;
                stream_data.packet_start = 0;
                if !args.send_null_packets {
                    if pid == 0x1FFF && is_mpegts {
                        continue;
                    }
                }
            } else if !args.extract_images && args.send_raw_stream {
                // Skip null packets
                if !args.send_null_packets {
                    if pid == 0x1FFF && is_mpegts {
                        // clear the Arc so it can be reused
                        stream_data.packet = Arc::new(Vec::new()); // Create a new Arc<Vec<u8>> for the next packet
                        continue;
                    }
                }
            }
            batch.push(stream_data);

            //info!("STATUS::PACKETS:CAPTURED: {}", packets_captured);
            // Check if batch is full
            if !args.no_zmq_thread {
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

    // Stop the pipeline when done
    #[cfg(feature = "gst")]
    match pipeline.set_state(gst::State::Null) {
        Ok(_) => (),
        Err(err) => {
            eprintln!("Failed to set the pipeline state to Null: {}", err);
        }
    }

    println!("\nWaiting for threads to finish...");

    // Send ZMQ stop signal
    tx.send(Vec::new()).await.unwrap();
    drop(tx);

    // Wait for the zmq_thread to finish
    capture_task.await.unwrap();
    zmq_thread.await.unwrap();

    println!("\nThreads finished, exiting rscap probe");
}
