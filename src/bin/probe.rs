/*
 * rscap: probe.rs - Rust Stream Capture with pcap, output serialized stats to ZeroMQ
 *
 * Written in 2024 by Chris Kennedy (C) LTN Global
 *
 * License: LGPL v2.1
 *
 */

use async_zmq;
use capnp;
use capnp::message::{Builder, HeapAllocator};
#[cfg(feature = "dpdk_enabled")]
use capsule::config::{load_config, DPDKConfig};
#[cfg(feature = "dpdk_enabled")]
use capsule::dpdk;
use clap::Parser;
use futures::stream::StreamExt;
use lazy_static::lazy_static;
use log::{debug, error, info};
use pcap::{Active, Capture, Device, PacketCodec};
use rscap::stream_data::{
    extract_pid, identify_video_pid, parse_and_store_pat, parse_pat, parse_pmt, tr101290_p1_check,
    tr101290_p2_check, Codec, PmtInfo, StreamData, Tr101290Errors, PAT_PID, TS_PACKET_SIZE,
};
use rscap::{current_unix_timestamp_ms, hexdump};
use rtp::RtpReader;
use rtp_rs as rtp;
use std::{
    collections::HashMap,
    error::Error as StdError,
    net::{IpAddr, Ipv4Addr, UdpSocket},
    sync::atomic::{AtomicBool, Ordering},
    sync::Arc,
    sync::Mutex,
    time::Instant,
};
use tokio::sync::mpsc::{self};
use zmq::PUB;
// Include the generated paths for the Cap'n Proto schema
include!("../stream_data_capnp.rs");

// Define your custom PacketCodec
pub struct BoxCodec;

impl PacketCodec for BoxCodec {
    type Item = Box<[u8]>;

    fn decode(&mut self, packet: pcap::Packet) -> Self::Item {
        packet.data.into()
    }
}

// global variable to store the MpegTS PID Map (initially empty)
lazy_static! {
    static ref PID_MAP: Mutex<HashMap<u16, Arc<StreamData>>> = Mutex::new(HashMap::new());
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

// Invoke this function for each MPEG-TS packet
fn process_packet(
    stream_data_packet: &mut StreamData,
    errors: &mut Tr101290Errors,
    is_mpegts: bool,
    pmt_pid: u16,
) {
    let packet: &[u8] = &stream_data_packet.packet[stream_data_packet.packet_start
        ..stream_data_packet.packet_start + stream_data_packet.packet_len];
    tr101290_p1_check(packet, errors);
    tr101290_p2_check(packet, errors);

    let pid = stream_data_packet.pid;
    let arrival_time = current_unix_timestamp_ms().unwrap_or(0);

    let mut pid_map = PID_MAP.lock().unwrap();

    // TODO: high debug level output, may need a flag specific to this dump
    //info!("PID Map Contents: {:#?}", pid_map);

    // Check if the PID map already has an entry for this PID
    match pid_map.get_mut(&pid) {
        Some(stream_data_arc) => {
            // Existing StreamData instance found, update it
            let mut stream_data = Arc::clone(stream_data_arc);
            Arc::make_mut(&mut stream_data).update_stats(packet.len(), arrival_time);
            Arc::make_mut(&mut stream_data).increment_count(1);
            if stream_data.pid != 0x1FFF && is_mpegts {
                Arc::make_mut(&mut stream_data)
                    .set_continuity_counter(stream_data_packet.continuity_counter);
            }
            let uptime = arrival_time - stream_data.start_time;

            // print out each field of structure
            debug!("STATUS::PACKET:MODIFY[{}] pid: {} stream_type: {} bitrate: {} bitrate_max: {} bitrate_min: {} bitrate_avg: {} iat: {} iat_max: {} iat_min: {} iat_avg: {} errors: {} continuity_counter: {} timestamp: {} uptime: {}", stream_data.pid, stream_data.pid, stream_data.stream_type, stream_data.bitrate, stream_data.bitrate_max, stream_data.bitrate_min, stream_data.bitrate_avg, stream_data.iat, stream_data.iat_max, stream_data.iat_min, stream_data.iat_avg, stream_data.error_count, stream_data.continuity_counter, stream_data.timestamp, uptime);

            stream_data_packet.bitrate = stream_data.bitrate;
            stream_data_packet.bitrate_avg = stream_data.bitrate_avg;
            stream_data_packet.bitrate_max = stream_data.bitrate_max;
            stream_data_packet.bitrate_min = stream_data.bitrate_min;
            stream_data_packet.iat = stream_data.iat;
            stream_data_packet.iat_avg = stream_data.iat_avg;
            stream_data_packet.iat_max = stream_data.iat_max;
            stream_data_packet.iat_min = stream_data.iat_min;
            stream_data_packet.stream_type = stream_data.stream_type.clone();
            stream_data_packet.start_time = stream_data.start_time;
            stream_data_packet.error_count = stream_data.error_count;
            stream_data_packet.last_arrival_time = stream_data.last_arrival_time;
            stream_data_packet.total_bits = stream_data.total_bits;
            stream_data_packet.count = stream_data.count;

            // write the stream_data back to the pid_map with modified values
            pid_map.insert(pid, stream_data);
        }
        None => {
            // No StreamData instance found for this PID, possibly no PMT yet
            if pmt_pid != 0xFFFF {
                debug!("ProcessPacket: New PID {} Found, adding to PID map.", pid);
            } else {
                // PMT packet not found yet, add the stream_data_packet to the pid_map
                let mut stream_data = Arc::new(StreamData::new(
                    Arc::new(Vec::new()), // Ensure packet_data is Arc<Vec<u8>>
                    0,
                    0,
                    stream_data_packet.pid,
                    stream_data_packet.stream_type.clone(),
                    stream_data_packet.start_time,
                    stream_data_packet.timestamp,
                    stream_data_packet.continuity_counter,
                ));
                Arc::make_mut(&mut stream_data).update_stats(packet.len(), arrival_time);

                // print out each field of structure
                info!("STATUS::PACKET:ADD[{}] pid: {} stream_type: {} bitrate: {} bitrate_max: {} bitrate_min: {} bitrate_avg: {} iat: {} iat_max: {} iat_min: {} iat_avg: {} errors: {} continuity_counter: {} timestamp: {} uptime: {}", stream_data.pid, stream_data.pid, stream_data.stream_type, stream_data.bitrate, stream_data.bitrate_max, stream_data.bitrate_min, stream_data.bitrate_avg, stream_data.iat, stream_data.iat_max, stream_data.iat_min, stream_data.iat_avg, stream_data.error_count, stream_data.continuity_counter, stream_data.timestamp, 0);

                pid_map.insert(pid, stream_data);
            }
        }
    }
}

// Use the stored PAT packet
fn update_pid_map(pmt_packet: &[u8], last_pat_packet: &[u8]) {
    let mut pid_map = PID_MAP.lock().unwrap();

    // Process the stored PAT packet to find program numbers and corresponding PMT PIDs
    let program_pids = last_pat_packet
        .chunks_exact(TS_PACKET_SIZE)
        .flat_map(parse_pat)
        .collect::<Vec<_>>();

    for pat_entry in program_pids.iter() {
        let program_number = pat_entry.program_number;
        let pmt_pid = pat_entry.pmt_pid;

        // Log for debugging
        debug!(
            "UpdatePIDmap: Processing Program Number: {}, PMT PID: {}",
            program_number, pmt_pid
        );

        // Ensure the current PMT packet matches the PMT PID from the PAT
        if extract_pid(pmt_packet) == pmt_pid {
            let pmt = parse_pmt(pmt_packet);

            for pmt_entry in pmt.entries.iter() {
                debug!(
                    "UpdatePIDmap: Processing PMT PID: {} for Stream PID: {} Type {}",
                    pmt_pid, pmt_entry.stream_pid, pmt_entry.stream_type
                );

                let stream_pid = pmt_entry.stream_pid;
                let stream_type = match pmt_entry.stream_type {
                    0x00 => "Reserved",
                    0x01 => "ISO/IEC 11172 MPEG-1 Video",
                    0x02 => "ISO/IEC 13818-2 MPEG-2 Video",
                    0x03 => "ISO/IEC 11172 MPEG-1 Audio",
                    0x04 => "ISO/IEC 13818-3 MPEG-2 Audio",
                    0x05 => "ISO/IEC 13818-1 Private Section",
                    0x06 => "ISO/IEC 13818-1 Private PES data packets",
                    0x07 => "ISO/IEC 13522 MHEG",
                    0x08 => "ISO/IEC 13818-1 Annex A DSM CC",
                    0x09 => "H222.1",
                    0x0A => "ISO/IEC 13818-6 type A",
                    0x0B => "ISO/IEC 13818-6 type B",
                    0x0C => "ISO/IEC 13818-6 type C",
                    0x0D => "ISO/IEC 13818-6 type D",
                    0x0E => "ISO/IEC 13818-1 auxillary",
                    0x0F => "13818-7 AAC Audio with ADTS transport syntax",
                    0x10 => "14496-2 Visual (MPEG-4 part 2 video)",
                    0x11 => "14496-3 MPEG-4 Audio with LATM transport syntax (14496-3/AMD 1)",
                    0x12 => "14496-1 SL-packetized or FlexMux stream in PES packets",
                    0x13 => "14496-1 SL-packetized or FlexMux stream in 14496 sections",
                    0x14 => "ISO/IEC 13818-6 Synchronized Download Protocol",
                    0x15 => "Metadata in PES packets",
                    0x16 => "Metadata in metadata_sections",
                    0x17 => "Metadata in 13818-6 Data Carousel",
                    0x18 => "Metadata in 13818-6 Object Carousel",
                    0x19 => "Metadata in 13818-6 Synchronized Download Protocol",
                    0x1A => "13818-11 MPEG-2 IPMP stream",
                    0x1B => "H.264/14496-10 video (MPEG-4/AVC)",
                    0x24 => "H.265 video (MPEG-H/HEVC)",
                    0x42 => "AVS Video",
                    0x7F => "IPMP stream",
                    0x81 => "ATSC A/52 AC-3",
                    0x86 => "SCTE 35 Splice Information Table",
                    0x87 => "ATSC A/52e AC-3",
                    _ if pmt_entry.stream_type < 0x80 => "ISO/IEC 13818-1 reserved",
                    _ => "User Private",
                };

                let timestamp = current_unix_timestamp_ms().unwrap_or(0);

                if !pid_map.contains_key(&stream_pid) {
                    let mut stream_data = Arc::new(StreamData::new(
                        Arc::new(Vec::new()), // Ensure packet_data is Arc<Vec<u8>>
                        0,
                        0,
                        stream_pid,
                        stream_type.to_string(),
                        timestamp,
                        timestamp,
                        0,
                    ));
                    // update stream_data stats
                    Arc::make_mut(&mut stream_data).update_stats(pmt_packet.len(), timestamp);

                    // print out each field of structure
                    info!("STATUS::STREAM:CREATE[{}] pid: {} stream_type: {} bitrate: {} bitrate_max: {} bitrate_min: {} bitrate_avg: {} iat: {} iat_max: {} iat_min: {} iat_avg: {} errors: {} continuity_counter: {} timestamp: {} uptime: {}", stream_data.pid, stream_data.pid, stream_data.stream_type, stream_data.bitrate, stream_data.bitrate_max, stream_data.bitrate_min, stream_data.bitrate_avg, stream_data.iat, stream_data.iat_max, stream_data.iat_min, stream_data.iat_avg, stream_data.error_count, stream_data.continuity_counter, stream_data.timestamp, 0);

                    pid_map.insert(stream_pid, stream_data);
                } else {
                    // get the stream data so we can update it
                    let stream_data_arc = pid_map.get_mut(&stream_pid).unwrap();
                    let mut stream_data = Arc::clone(stream_data_arc);

                    // update the stream type
                    Arc::make_mut(&mut stream_data).update_stream_type(stream_type.to_string());

                    // print out each field of structure
                    debug!("STATUS::STREAM:UPDATE[{}] pid: {} stream_type: {} bitrate: {} bitrate_max: {} bitrate_min: {} bitrate_avg: {} iat: {} iat_max: {} iat_min: {} iat_avg: {} errors: {} continuity_counter: {} timestamp: {} uptime: {}", stream_data.pid, stream_data.pid, stream_data.stream_type, stream_data.bitrate, stream_data.bitrate_max, stream_data.bitrate_min, stream_data.bitrate_avg, stream_data.iat, stream_data.iat_max, stream_data.iat_min, stream_data.iat_avg, stream_data.error_count, stream_data.continuity_counter, stream_data.timestamp, 0);

                    // write the stream_data back to the pid_map with modified values
                    pid_map.insert(stream_pid, stream_data);
                }
            }
        } else {
            error!("UpdatePIDmap: Skipping PMT PID: {} as it does not match with current PMT packet PID", pmt_pid);
        }
    }
}

fn determine_stream_type(pid: u16) -> String {
    let pid_map = PID_MAP.lock().unwrap();

    // check if pid already is mapped, if so return the stream type already stored
    if let Some(stream_data) = pid_map.get(&pid) {
        return stream_data.stream_type.clone();
    }

    pid_map
        .get(&pid)
        .map(|stream_data| stream_data.stream_type.clone())
        .unwrap_or_else(|| "unknown".to_string())
}

// Define a custom error for when the target device is not found
#[derive(Debug)]
struct DeviceNotFoundError;

impl std::fmt::Display for DeviceNotFoundError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Target device not found")
    }
}

impl StdError for DeviceNotFoundError {}

#[cfg(all(feature = "dpdk_enabled", target_os = "linux"))]
fn init_dpdk(port_id: u16) -> Result<dpdk::eal::Port, Box<dyn std::error::Error>> {
    // DPDK initialization logic here
    let config = load_config()?;
    info!("DPDK config: {:?} setup on port {}", config, port_id);
    dpdk::eal::init(config)?;

    // Configure and start the network interface
    let port = dpdk::eal::Port::new(port_id)?;
    port.configure()?;
    port.start()?;

    // Set promiscuous mode if needed
    if promiscuous_mode {
        port.set_promiscuous(true)?;
    }

    info!("DPDK port {} initialized", port_id);

    Ok(port)
}

// Placeholder for non-Linux or DPDK disabled builds
#[cfg(not(all(feature = "dpdk_enabled", target_os = "linux")))]
fn init_dpdk(_port_id: u16) -> Result<(), Box<dyn std::error::Error>> {
    Err("DPDK is not supported on this OS".into())
}

fn init_pcap(
    source_device: &str,
    use_wireless: bool,
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
    version = "1.1",
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

    /// DPDK enable
    #[clap(long, env = "DPDK", default_value_t = false)]
    dpdk: bool,

    /// DPDK Port ID
    #[clap(long, env = "DPDK_PORT_ID", default_value_t = 0)]
    dpdk_port_id: u16,

    /// IPC Path for ZeroMQ
    #[clap(long, env = "IPC_PATH")]
    ipc_path: Option<String>,
}

// MAIN Function
#[tokio::main]
async fn main() {
    println!("RsCap Probe for ZeroMQ output of MPEG-TS and SMPTE 2110 streams from pcap.");

    dotenv::dotenv().ok(); // read .env file

    let args = Args::parse();

    // Use the parsed arguments directly
    let batch_size = args.batch_size;
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
    let _send_raw_stream = args.send_raw_stream;
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

    if args.smpte2110 {
        // --batch-size 1 --buffer-size 10000000000 --pcap-channel-size 100000 --zmq-channel-size 10000 --packet-size 1208 --immediate-mode
        buffer_size = 10000000000; // set buffer size to 10GB for smpte2110
        pcap_channel_size = 100_000; // set pcap channel size to 100000 for smpte2110
        zmq_channel_size = 10_000; // set zmq channel size to 10000 for smpte2110
        packet_size = 1208; // set packet size to 1208 for smpte2110
        immediate_mode = true; // set immediate mode to true for smpte2110
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
    let running_clone = running.clone();

    // Spawn a new thread for packet capture
    let capture_task = if cfg!(feature = "dpdk_enabled") && args.dpdk {
        // DPDK is enabled
        tokio::spawn(async move {
            // TODO: DPDK initialization logic goes here");
            error!("DPDK is not yet implemented");
            // Implement DPDK initialization logic here
            match init_dpdk(args.dpdk_port_id) {
                Ok(()) => {
                    let mut count = 0;
                    // DPDK packet processing loop
                    while running_clone.load(Ordering::SeqCst) {
                        // Fetch and process packets using DPDK
                        // ...

                        count += 1;
                        if debug_on {
                            // Print stats or debug information
                            println!("DPDK packet count: {}", count);
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to initialize DPDK: {}", e);
                }
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

            while running_clone.load(Ordering::SeqCst) {
                while let Some(packet) = stream.next().await {
                    match packet {
                        Ok(data) => {
                            count += 1;
                            let packet_data = Arc::new(data.to_vec());
                            ptx.send(packet_data).await.unwrap();
                            let current_ts = Instant::now();
                            if pcap_stats
                                && ((current_ts.duration_since(stats_last_sent_ts).as_secs() >= 30)
                                    || count == 1)
                            {
                                stats_last_sent_ts = current_ts;
                                let stats = stream.capture_mut().stats().unwrap();
                                println!(
                                    "#{} Current stats: Received: {}, Dropped: {}, Interface Dropped: {}",
                                    count, stats.received, stats.dropped, stats.if_dropped
                                );
                            }
                        }
                        Err(e) => {
                            // Print error and information about it
                            error!("PCap Capture Error occurred: {}", e);
                            if e == pcap::Error::TimeoutExpired {
                                // Timeout expired, continue and try again
                                continue;
                            } else {
                                // Exit the loop if an error occurs
                                running_clone.store(false, Ordering::SeqCst);
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
            }

            let stats = stream.capture_mut().stats().unwrap();
            println!("Packet capture statistics:");
            println!("Received: {}", stats.received);
            println!("Dropped: {}", stats.dropped);
            println!("Interface Dropped: {}", stats.if_dropped);
        })
    };

    // Setup channel for passing data between threads
    let (tx, mut rx) = mpsc::channel::<Vec<StreamData>>(zmq_channel_size);

    // Spawn a new thread for ZeroMQ communication
    let zmq_thread = tokio::spawn(async move {
        let context = async_zmq::Context::new();
        let publisher = context.socket(PUB).unwrap();
        // Determine the connection endpoint (IPC if provided, otherwise TCP)
        let endpoint = if let Some(ipc_path_copy) = args.ipc_path {
            format!("ipc://{}", ipc_path_copy)
        } else {
            format!("tcp://{}:{}", target_ip, target_port)
        };

        info!("ZeroMQ publisher startup {}", endpoint);

        publisher.bind(&endpoint).unwrap();

        while let Some(mut batch) = rx.recv().await {
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
                /*let packet_slice = &stream_data.packet
                [stream_data.packet_start..stream_data.packet_start + stream_data.packet_len];*/
                /*let mut packet_msg = zmq::Message::from(Vec::new());
                if send_raw_stream {
                    packet_msg = zmq::Message::from(packet_slice);
                    }*/
                publisher.send(capnp_msg, 0 /*zmq::SNDMORE*/).unwrap();
                //publisher.send(packet_msg, 0).unwrap();
            }
            batch.clear();
        }
    });

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

    while let Some(packet) = prx.recv().await {
        if packet_count > 0 && packets_captured > packet_count {
            running.store(false, Ordering::SeqCst);
            break;
        }
        packets_captured += 1;

        if !no_progress {
            print!(".");
            // flush stdout (wastes resources) TODO: output in ncurses or something w/less output
            //std::io::stdout().flush().unwrap();
        }

        // Check if chunk is MPEG-TS or SMPTE 2110
        let chunk_type = is_mpegts_or_smpte2110(&packet[payload_offset..]);
        if chunk_type != 1 {
            if chunk_type == 0 {
                error!("Not MPEG-TS or SMPTE 2110");
                hexdump(&packet, 0, packet.len());
            }
            is_mpegts = false;
        }

        let chunks = if is_mpegts {
            process_mpegts_packet(payload_offset, &packet, packet_size, start_time)
        } else {
            process_smpte2110_packet(payload_offset, &packet, packet.len(), start_time)
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

            if pid == 0x1FFF && is_mpegts {
                // clear the Arc so it can be reused
                stream_data.packet = Arc::new(Vec::new()); // Create a new Arc<Vec<u8>> for the next packet
                                                           // Skip null packets
                continue;
            }
            batch.push(stream_data);

            //info!("STATUS::PACKETS:CAPTURED: {}", packets_captured);
            // Check if batch is full
            if !no_zmq {
                if batch.len() >= batch_size {
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
    /*Err(e) => {
    error!("Error capturing packet: {:?}", e);
    }*/

    println!("Exiting rscap probe");

    // Send stop signal
    tx.send(Vec::new()).await.unwrap();
    drop(tx);

    // Wait for the zmq_thread to finish
    zmq_thread.await.unwrap();
    capture_task.await.unwrap();
}

// Check if the packet is MPEG-TS or SMPTE 2110
fn is_mpegts_or_smpte2110(packet: &[u8]) -> i32 {
    // Check for MPEG-TS (starts with 0x47 sync byte)
    if packet.starts_with(&[0x47]) {
        return 1;
    }

    // Basic check for RTP (which SMPTE ST 2110 uses)
    // This checks if the first byte is 0x80 or 0x81
    // This might need more robust checks based on requirements
    if packet.len() > 12 && (packet[0] == 0x80 || packet[0] == 0x81) {
        // TODO: Check payload type or other RTP header fields here if necessary
        return 2; // Assuming it's SMPTE ST 2110 for now
    }

    0 // Not MPEG-TS or SMPTE 2110
}

// ## RFC 4175 SMPTE2110 header functions ##
/*const RFC_4175_EXT_SEQ_NUM_LEN: usize = 2;
const RFC_4175_HEADER_LEN: usize = 6; // Note: extended sequence number not included*/ // TODO: implement RFC 4175 SMPTE2110 header functions

fn get_extended_sequence_number(buf: &[u8]) -> u16 {
    ((buf[0] as u16) << 8) | buf[1] as u16
}

fn get_line_length(buf: &[u8]) -> u16 {
    ((buf[0] as u16) << 8) | buf[1] as u16
}

fn get_line_field_id(buf: &[u8]) -> u8 {
    buf[2] >> 7
}

fn get_line_number(buf: &[u8]) -> u16 {
    ((buf[2] as u16 & 0x7f) << 8) | buf[3] as u16
}

fn get_line_continuation(buf: &[u8]) -> u8 {
    buf[4] >> 7
}

fn get_line_offset(buf: &[u8]) -> u16 {
    ((buf[4] as u16 & 0x7f) << 8) | buf[5] as u16
}
// ## End of RFC 4175 SMPTE2110 header functions ##

// Process the packet and return a vector of SMPTE ST 2110 packets
fn process_smpte2110_packet(
    payload_offset: usize,
    packet: &Arc<Vec<u8>>,
    packet_size: usize,
    start_time: u64,
) -> Vec<StreamData> {
    let mut streams = Vec::new();

    // Check if the packet is large enough to contain an RTP header
    if packet_size > payload_offset + 12 {
        // Check for RTP header marker
        if packet[payload_offset] == 0x80 || packet[payload_offset] == 0x81 {
            let rtp_packet = &packet[payload_offset..];

            // Create an RtpReader
            if let Ok(rtp) = RtpReader::new(rtp_packet) {
                // Extract the timestamp and payload type
                let timestamp = rtp.timestamp();
                let payload_type = rtp.payload_type();

                // Calculate the actual start of the RTP payload
                let rtp_payload_offset = payload_offset + rtp.payload_offset();

                // Calculate the length of the RTP payload
                let rtp_payload_length = packet_size - rtp_payload_offset;

                // Extract SMPTE 2110 specific fields
                let line_length = get_line_length(rtp_packet);
                let line_number = get_line_number(rtp_packet);
                let extended_sequence_number = get_extended_sequence_number(rtp_packet);
                let line_offset = get_line_offset(rtp_packet);
                let field_id = get_line_field_id(rtp_packet);

                let line_continuation = get_line_continuation(rtp_packet);

                // Use payload type as PID (for the purpose of this example)
                let pid = payload_type as u16;
                let stream_type = payload_type.to_string();

                // Create new StreamData instance
                let mut stream_data = StreamData::new(
                    Arc::clone(packet),
                    rtp_payload_offset,
                    rtp_payload_length,
                    pid,
                    stream_type,
                    start_time,
                    timestamp as u64,
                    0,
                );

                // Update StreamData stats and RTP fields
                stream_data
                    .update_stats(rtp_payload_length, current_unix_timestamp_ms().unwrap_or(0));
                stream_data.set_rtp_fields(
                    timestamp,
                    payload_type,
                    payload_type.to_string(),
                    line_number,
                    line_offset,
                    line_length,
                    field_id,
                    line_continuation,
                    extended_sequence_number,
                );

                // Add the StreamData to the stream list
                streams.push(stream_data);
            } else {
                hexdump(&packet, 0, packet_size);
                error!("Error parsing RTP header, not SMPTE ST 2110");
            }
        } else {
            hexdump(&packet, 0, packet_size);
            error!("No RTP header detected, not SMPTE ST 2110");
        }
    } else {
        hexdump(&packet, 0, packet_size);
        error!("Packet too small, not SMPTE ST 2110");
    }

    streams
}

// Process the packet and return a vector of MPEG-TS packets
fn process_mpegts_packet(
    payload_offset: usize,
    packet: &Arc<Vec<u8>>,
    packet_size: usize,
    start_time: u64,
) -> Vec<StreamData> {
    let mut start = payload_offset;
    let mut read_size = packet_size;
    let mut streams = Vec::new();

    let len = packet.len();

    while start + read_size <= len {
        let chunk = &packet[start..start + read_size];
        if chunk[0] == 0x47 {
            // Check for MPEG-TS sync byte
            read_size = packet_size; // reset read_size

            let pid = extract_pid(chunk);

            let stream_type = determine_stream_type(pid); // Implement this function based on PAT/PMT parsing
            let timestamp = ((chunk[4] as u64) << 25)
                | ((chunk[5] as u64) << 17)
                | ((chunk[6] as u64) << 9)
                | ((chunk[7] as u64) << 1)
                | ((chunk[8] as u64) >> 7);
            let continuity_counter = chunk[3] & 0x0F;

            let mut stream_data = StreamData::new(
                Arc::clone(packet),
                start,
                packet_size,
                pid,
                stream_type,
                start_time,
                timestamp,
                continuity_counter,
            );
            stream_data.update_stats(packet_size, current_unix_timestamp_ms().unwrap_or(0));
            streams.push(stream_data);
        } else {
            error!("ProcessPacket: Not MPEG-TS");
            hexdump(&packet, start, packet_size);
            read_size = 1; // Skip to the next byte
        }
        start += read_size;
    }

    streams
}
