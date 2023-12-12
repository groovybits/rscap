/*
 * rscap: probe - Rust Stream Capture with pcap, output to ZeroMQ
 *
 * Written in 2023 by Chris Kennedy (C) LTN Global
 *
 * License: LGPL v2.1
 *
 */

extern crate zmq;
extern crate rtp_rs as rtp;
use rtp::RtpReader;
use pcap::{Capture};
use serde_json::json;
use log::{error, debug, info};
use tokio;
use std::net::{Ipv4Addr, UdpSocket};
use std::env;
use std::io::Write;
use std::sync::mpsc;
use std::thread;
use std::collections::HashMap;
use std::sync::Mutex;
use lazy_static::lazy_static;
use std::time::{SystemTime, UNIX_EPOCH};

// constant for PAT PID
const PAT_PID: u16 = 0;

// global variable to store PMT PID (initially set to an invalid PID)
static mut PMT_PID: u16 = 0xFFFF;

static mut LAST_PAT_PACKET: Option<Vec<u8>> = None;

lazy_static! {
    static ref PID_MAP: Mutex<HashMap<u16, StreamData>> = Mutex::new(HashMap::new());
}

struct PatEntry {
    program_number: u16,
    pmt_pid: u16,
}

struct PmtEntry {
    stream_pid: u16,
    stream_type: u8, // Stream type (e.g., 0x02 for MPEG video)
}

struct Pmt {
    entries: Vec<PmtEntry>,
}

// StreamData struct
#[derive(Clone)]
struct StreamData {
    pid: u16,
    stream_type: String, // "video", "audio", "text"
    continuity_counter: u8,
    timestamp: u64,
    bitrate: u32,
    iat: u64,
    error_count: u32,
    last_arrival_time: u64,
    start_time: u64,        // field for start time
    total_bits: u64,        // field for total bits
    data: Vec<u8>, // The actual MPEG-TS packet data
}

// StreamData implementation
impl StreamData {
    fn new(packet: &[u8], pid: u16, stream_type: String, start_time: u64, timestamp: u64, continuity_counter: u8) -> Self {      
        let bitrate = 0;
        let iat = 0;
        let error_count = 0;
        let last_arrival_time = current_unix_timestamp_ms().unwrap_or(0);
        StreamData {
            pid,
            stream_type,
            continuity_counter,
            timestamp,
            bitrate,
            iat,
            error_count,
            last_arrival_time,
            start_time,         // Initialize start time
            total_bits: 0,                 // Initialize total bits
            data: packet.to_vec(),
        }
    }
    fn update_stats(&mut self, packet_size: usize, arrival_time: u64) {
        let bits = packet_size as u64 * 8; // Convert bytes to bits

        // Elapsed time in milliseconds
        let elapsed_time_ms = arrival_time - self.start_time;

        if elapsed_time_ms > 0 {
            let elapsed_time_sec = elapsed_time_ms as f64 / 1000.0;
            self.bitrate = (self.total_bits as f64 / elapsed_time_sec) as u32;
        }

        self.total_bits += bits; // Accumulate total bits

        // IAT calculation remains the same
        let iat = arrival_time - self.last_arrival_time;
        self.iat = iat;

        self.last_arrival_time = arrival_time;
    }
}

struct Tr101290Errors {
    sync_byte_errors: u32,
    transport_error_indicator_errors: u32,
    continuity_counter_errors: u32,
    // ... other error types ...
}

impl Tr101290Errors {
    fn new() -> Self {
        Tr101290Errors {
            sync_byte_errors: 0,
            transport_error_indicator_errors: 0,
            continuity_counter_errors: 0,
            // ... initialize other errors ...
        }
    }

    fn log_errors(&self) {
        // Log the error counts for monitoring
        if self.sync_byte_errors > 0 {
            error!("Sync byte errors: {}", self.sync_byte_errors);
        }
        if self.transport_error_indicator_errors > 0 {
            error!("Transport Error Indicator errors: {}", self.transport_error_indicator_errors);
        }
        if self.continuity_counter_errors > 0 {
            error!("Continuity counter errors: {}", self.continuity_counter_errors);
        }
        // ... log other errors ...
    }
}

// TR 101 290 Priority 1 Check Example
fn tr101290_p1_check(packet: &[u8], errors: &mut Tr101290Errors) {
    if packet[0] != 0x47 {
        errors.sync_byte_errors += 1;
    }

    if (packet[1] & 0x80) != 0 {
        errors.transport_error_indicator_errors += 1;
    }

    // ... other checks, updating the respective counters ...
}

// Invoke this function for each MPEG-TS packet
fn process_packet(packet: &[u8], errors: &mut Tr101290Errors) {
    tr101290_p1_check(packet, errors);
}

// Function to get the current Unix timestamp in milliseconds
fn current_unix_timestamp_ms() -> Result<u64, String> {
    let now = SystemTime::now();
    match now.duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            let milliseconds = duration.as_secs() * 1000 + u64::from(duration.subsec_millis());
            Ok(milliseconds)
        }
        Err(e) => Err(format!("System time is before the UNIX epoch: {}", e)),
    }
}

// Implement a function to extract PID from a packet
fn extract_pid(packet: &[u8]) -> u16 {
    // Extract PID from packet
    // (You'll need to adjust the indices according to your packet format)
    ((packet[1] as u16 & 0x1F) << 8) | packet[2] as u16
}

// Helper function to parse PAT and update global PAT packet storage
fn parse_and_store_pat(packet: &[u8]) {
    let pat_entries = parse_pat(packet);
    unsafe {
        // Store the specific PAT chunk for later use
        LAST_PAT_PACKET = Some(packet.to_vec());
    }

    // Assuming there's only one program for simplicity, update PMT PID
    if let Some(first_entry) = pat_entries.first() {
        unsafe { PMT_PID = first_entry.pmt_pid };
    }
}

fn parse_pat(packet: &[u8]) -> Vec<PatEntry> {
    let mut entries = Vec::new();

    // Check if Payload Unit Start Indicator (PUSI) is set
    let pusi = (packet[1] & 0x40) != 0;
    let adaptation_field_control = (packet[3] & 0x30) >> 4;
    let mut pat_start = 4; // start after TS header
    
    if adaptation_field_control == 0x02 || adaptation_field_control == 0x03 {
        let adaptation_field_length = packet[4] as usize;
        pat_start += 1 + adaptation_field_length; // +1 for the length byte itself
        debug!("ParsePAT: Adaptation Field Length: {}", adaptation_field_length);
    } else {
        debug!("ParsePAT: Skipping Adaptation Field Control: {}", adaptation_field_control);
    }

    if pusi {
        // PUSI is set, so the first byte after the TS header is the pointer field
        let pointer_field = packet[pat_start] as usize;
        //pat_start += 1 + pointer_field; // Skip pointer field
        debug!("ParsePAT: PUSI set as {}, skipping pointer field {} as packet value {} {}", pusi, pointer_field, packet[0], packet[1]);
    } else {
        debug!("ParsePAT: PUSI not set as {}", pusi);
    }

    debug!("ParsePAT: PAT start: {}", pat_start);

    // Check for the presence of a pointer field
    let pointer_field = packet[pat_start] as usize;
    pat_start += 1 + pointer_field; // Move past the pointer field

    let mut i = pat_start; // Starting index of the PAT data

    while i + 4 <= packet.len() {
        let program_number = ((packet[i] as u16) << 8) | (packet[i + 1] as u16);
        // Mask the lower 13 bits for the PMT PID
        let pmt_pid = (((packet[i + 2] as u16) & 0x1F) << 8) | (packet[i + 3] as u16);

        //debug!("ParsePAT: Packet1 {}, Packet2 {}, Packet3 {}, Packet4 {}", packet[i], packet[i + 1], packet[i + 2], packet[i + 3]);

        if program_number != 0 && program_number != 65535 && pmt_pid != 0 && program_number < 30 /* FIXME: kludge fix for now */ {
            debug!("ParsePAT: Program Number: {} PMT PID: {}", program_number, pmt_pid);
            entries.push(PatEntry { program_number, pmt_pid });
        }

        i += 4;
    }

    entries
}

fn parse_pmt(packet: &[u8], pmt_pid: u16) -> Pmt {
    let mut entries = Vec::new();
    let program_number = ((packet[8] as u16) << 8) | (packet[9] as u16);

    // Calculate the starting position for stream entries
    let section_length = (((packet[6] as usize) & 0x0F) << 8) | packet[7] as usize;
    let program_info_length = (((packet[15] as usize) & 0x0F) << 8) | packet[16] as usize;
    let mut i = 17 + program_info_length; // Starting index of the first stream in the PMT

    debug!("ParsePMT: Program Number: {} PMT PID: {} starting at position {}", program_number, pmt_pid, i);
    while i + 5 <= packet.len() && i < 17 + section_length - 4 {
        let stream_type = packet[i];
        let stream_pid = (((packet[i + 1] as u16) & 0x1F) << 8) | (packet[i + 2] as u16);
        let es_info_length = (((packet[i + 3] as usize) & 0x0F) << 8) | packet[i + 4] as usize;
        i += 5 + es_info_length; // Update index to point to next stream's info

        entries.push(PmtEntry { stream_pid, stream_type });
        debug!("ParsePMT: Stream PID: {}, Stream Type: {}", stream_pid, stream_type);
    }

    Pmt { entries }
}

// Modify the function to use the stored PAT packet
fn update_pid_map(pmt_packet: &[u8]) {
    let mut pid_map = PID_MAP.lock().unwrap();

    // Process the stored PAT packet to find program numbers and corresponding PMT PIDs
    let program_pids = unsafe {
        LAST_PAT_PACKET.as_ref().map_or_else(Vec::new, |pat_packet| parse_pat(pat_packet))
    };

    for pat_entry in program_pids.iter() {
        let program_number = pat_entry.program_number;
        let pmt_pid = pat_entry.pmt_pid;

        // Log for debugging
        debug!("UpdatePIDmap: Processing Program Number: {}, PMT PID: {}", program_number, pmt_pid);

        // Ensure the current PMT packet matches the PMT PID from the PAT
        if extract_pid(pmt_packet) == pmt_pid {
            let pmt = parse_pmt(pmt_packet, pmt_pid);

            for pmt_entry in pmt.entries.iter() {
                debug!("UpdatePIDmap: Processing PMT PID: {} for Stream PID: {} Type {}", pmt_pid, pmt_entry.stream_pid, pmt_entry.stream_type);

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

                debug!("UpdatePIDmap: Added Stream PID: {}, Stream Type: {}/{}", stream_pid, pmt_entry.stream_type, stream_type);
                let stream_data = StreamData::new(&[], stream_pid, stream_type.to_string(), timestamp, timestamp, 0);
                pid_map.insert(stream_pid, stream_data);
            }
        } else {
            error!("UpdatePIDmap: Skipping PMT PID: {} as it does not match with current PMT packet PID", pmt_pid);
        }
    }
}

fn determine_stream_type(pid: u16) -> String {
    let pid_map = PID_MAP.lock().unwrap();
    pid_map.get(&pid)
           .map(|stream_data| stream_data.stream_type.clone())
           .unwrap_or_else(|| "unknown".to_string())
}

// MAIN
#[tokio::main]
async fn main() {
    info!("Starting rscap probe");
    dotenv::dotenv().ok(); // read .env file

    // setup various read/write variables
    let mut batch_size: usize = env::var("BATCH_SIZE").unwrap_or("1000".to_string()).parse().expect(&format!("Invalid format for BATCH_SIZE"));
    let payload_offset: usize = env::var("PAYLOAD_OFFSET").unwrap_or("42".to_string()).parse().expect(&format!("Invalid format for PAYLOAD_OFFSET"));
    let packet_size: usize = env::var("PACKET_SIZE").unwrap_or("188".to_string()).parse().expect(&format!("Invalid format for PACKET_SIZE"));
    let read_time_out: i32 = env::var("READ_TIME_OUT").unwrap_or("300000".to_string()).parse().expect(&format!("Invalid format for READ_TIME_OUT"));

    // calculate read size based on batch size and packet size
    let read_size: i32 = (packet_size as i32 * batch_size as i32) + payload_offset as i32; // pcap read size

    // Get environment variables or use default values, set in .env file
    let target_port: i32 = env::var("TARGET_PORT").unwrap_or("5556".to_string()).parse().expect(&format!("Invalid format for TARGET_PORT"));
    let target_ip: String = env::var("TARGET_IP").unwrap_or("127.0.0.1".to_string());
    let source_device: String = env::var("SOURCE_DEVICE").unwrap_or("".to_string());
    let source_ip: String = env::var("SOURCE_IP").unwrap_or("224.0.0.200".to_string());
    let source_protocol: String = env::var("SOURCE_PROTOCOL").unwrap_or("udp".to_string());

    let source_port: i32 = env::var("SOURCE_PORT").unwrap_or("10000".to_string()).parse().expect(&format!("Invalid format for SOURCE_PORT"));
    let source_device_ip: &str = "0.0.0.0";

    let debug_on: bool = env::var("DEBUG").unwrap_or("false".to_string()).parse().expect(&format!("Invalid format for DEBUG"));
    let silent: bool = env::var("SILENT").unwrap_or("false".to_string()).parse().expect(&format!("Invalid format for SILENT"));

    #[cfg(not(target_os = "linux"))]
    let use_wireless: bool = env::var("USE_WIRELESS").unwrap_or("false".to_string()).parse().expect(&format!("Invalid format for USE_WIRELESS"));

    let send_json_header: bool = env::var("SEND_JSON_HEADER").unwrap_or("false".to_string()).parse().expect(&format!("Invalid format for SEND_JSON_HEADER"));

    let mut is_mpegts = true; // Default to true, update based on actual packet type

    // Initialize logging
    env_logger::init(); // set RUST_LOG environment variable to debug for more verbose logging

    // device ip address
    let mut interface_addr = source_device_ip.parse::<Ipv4Addr>()
        .expect(&format!("Invalid IP address format in source_device_ip {}", source_device_ip));

    // Get the selected device's details
    let mut target_device_found = false;
    let devices = pcap::Device::list().unwrap();
    // List all devices and their flags
    for device in &devices {
        info!("Device: {:?}, Flags: {:?}", device.name, device.flags);
    }
    #[cfg(target_os = "linux")]
    let mut target_device = devices.clone().into_iter()
        .find(|d| d.name != "lo") // Exclude loopback device
        .expect("No valid devices found");

    #[cfg(not(target_os = "linux"))]
    let mut target_device = devices.clone().into_iter()
        .find(|d| d.flags.is_up() && !d.flags.is_loopback() && d.flags.is_running() && (!d.flags.is_wireless() || use_wireless))
        .expect(&format!("No valid devices found {}", devices.len()));

    info!("Default device: {:?}", target_device.name);

    // If source_device is auto, find the first valid device
    if source_device == "auto" || source_device == "" {
        info!("Auto-selecting device...");

        // Find the first valid device
        for device in pcap::Device::list().unwrap() {
            debug!("Device {:?}", device);

            // check flags for device up
            #[cfg(not(target_os = "linux"))]
            if !device.flags.is_up() {
                continue;
            }
            // check if device is loopback
            #[cfg(not(target_os = "linux"))]
            if device.flags.is_loopback() {
                continue;
            }
            // check if device is ethernet
            #[cfg(not(target_os = "linux"))]
            if device.flags.is_wireless() {
                if !use_wireless {
                    continue;
                }
            }
            // check if device is running
            #[cfg(not(target_os = "linux"))]
            if !device.flags.is_running() {
                continue;
            }
            
            // check if device has an IPv4 address
            for addr in device.addresses.iter() {
                if let std::net::IpAddr::V4(ipv4_addr) = addr.addr {
                    // check if loopback
                    if ipv4_addr.is_loopback() {
                        continue;
                    }
                    target_device_found = true;

                    // Found through auto-detection, set interface_addr
                    info!("Found IPv4 target device {} with ip {}", source_device, ipv4_addr);
                    interface_addr = ipv4_addr;
                    target_device = device;
                    break;
                }
            }
            // break out of loop if target device is found
            if target_device_found {
                break;
            }   
        }
    } else {
        // Use the specified device instead of auto-detection
        info!("Using specified device {}", source_device);

        // Find the specified device
        #[cfg(not(target_os = "linux"))]
        let target_device_discovered = devices.into_iter().find(|d| d.name == source_device && d.flags.is_up() && d.flags.is_running() && (!d.flags.is_wireless() || use_wireless))
            .expect(&format!("Target device not found {}", source_device));

        #[cfg(target_os = "linux")]
        let target_device_discovered = devices.into_iter().find(|d| d.name == source_device)
            .expect(&format!("Target device not found {}", source_device));

        // Check if device has an IPv4 address
        info!("Target Device: {:?}", target_device_discovered);
        for addr in target_device_discovered.addresses.iter() {
            if let std::net::IpAddr::V4(ipv4_addr) = addr.addr {
                info!("Found ipv4_addr: {:?}", ipv4_addr);
                interface_addr = ipv4_addr;
                target_device_found = true;
                target_device = target_device_discovered;
                break;
            }
        }
    }

    // Device not found
    if !target_device_found {
        error!("Target device {} not found", source_device);
        return;
    }

    // Join multicast group
    let multicast_addr = source_ip.parse::<Ipv4Addr>()
        .expect(&format!("Invalid IP address format in source_ip {}", source_ip));

    let socket = UdpSocket::bind("0.0.0.0:0").expect("Failed to bind socket");
    socket.join_multicast_v4(&multicast_addr, &interface_addr)
    .expect(&format!("Failed to join multicast group on interface {}", source_device));

    #[cfg(not(target_os = "linux"))]
    let promiscuous: bool = false;

    #[cfg(target_os = "linux")]
    let promiscuous: bool = true;

    // Setup packet capture
    let mut cap = Capture::from_device(target_device).unwrap()
        .promisc(promiscuous)
        .timeout(read_time_out)
        .snaplen(read_size) // Adjust this based on network configuration
        .open().unwrap();

    // Filter pcap
    let source_host_and_port = format!("{} dst port {} and ip dst host {}", source_protocol, source_port, source_ip);
    cap.filter(&source_host_and_port, true).unwrap();

    // Setup channel for passing data between threads
    let (tx, rx) = mpsc::channel::<Vec<Vec<u8>>>();

    // Spawn a new thread for ZeroMQ communication
    let zmq_thread = thread::spawn(move || {
        // Setup ZeroMQ publisher
        let context = zmq::Context::new();
        let publisher = context.socket(zmq::PUB).unwrap();
        let source_port_ip = format!("tcp://{}:{}", target_ip, target_port);
        publisher.bind(&source_port_ip).unwrap();

        let mut total_bytes = 0;
        let mut count = 0;
        for batch in rx {
            // Check for a stop signal
            if batch.is_empty() {
                break; // Exit the loop if a stop signal is received
            }
            // ... ZeroMQ sending logic ...
            let batched_data = batch.concat();

            let mut format_str = "unknown";
            let format_index = is_mpegts_or_smpte2110(&batched_data);
            if format_index == 1 {
                format_str = "mpegts";
            } else if format_index == 2 {
                format_str = "smpte2110";
            }
            // Check if JSON header is enabled
            if send_json_header {
                // Construct JSON header for batched data
                let json_header = json!({
                    "type": "mpegts_chunk",
                    "content_length": batched_data.len(),
                    "total_bytes": total_bytes,
                    "count": count,
                    "source_ip": source_ip,
                    "source_port": source_port,
                    "source_device": source_device,
                    "target_ip": target_ip,
                    "target_port": target_port,
                    "format": format_str,
                });

                // Send JSON header as multipart message
                publisher.send(json_header.to_string().as_bytes(), zmq::SNDMORE).unwrap();
            }

            // Send chunk of data as multipart message
            let chunk_size = batched_data.len();
            total_bytes += chunk_size;
            count += 1;
            publisher.send(batched_data, 0).unwrap();
            
            // Print progress
            if !debug_on && !silent{
                print!(".");
                // flush stdout
                std::io::stdout().flush().unwrap();
            } else if !silent {
                debug!("#{} Sent chunk of {}/{} bytes", count, chunk_size, total_bytes);
            }
        }
    });

    // Perform TR 101 290 checks
    let mut tr101290_errors = Tr101290Errors::new();

    // start time
    let start_time = current_unix_timestamp_ms().unwrap_or(0);

    // Start packet capture
    let mut batch = Vec::new();
    loop {
        match cap.next_packet() {
            Ok(packet) => {
                if debug_on{
                    debug!("Received packet! {:?}", packet.header);
                }

                // Check if chunk is MPEG-TS or SMPTE 2110
                let chunk_type = is_mpegts_or_smpte2110(&packet[payload_offset..]);
                if chunk_type != 1 {
                    debug!("Not MPEG-TS, type {}", chunk_type);
                    if chunk_type == 0 {
                        error!("Not MPEG-TS or SMPTE 2110");
                        hexdump(&packet);
                    }
                    is_mpegts = false;
                }

                let chunks = if is_mpegts {
                    process_mpegts_packet(payload_offset, &packet, packet_size, start_time)
                } else {
                    process_smpte2110_packet(payload_offset, &packet, packet_size, start_time)
                };

                if !is_mpegts {
                    batch_size = 1; // Set batch size to 1 for SMPTE 2110
                }

                // Process each chunk
                for stream_data in chunks {
                    if debug_on {
                        hexdump(&stream_data.data); // Use stream_data.data to access the raw packet data
                    }

                    // Check for TR 101 290 errors, skip for SMPTE 2110
                    if is_mpegts {
                        // Check for TR 101 290 errors
                        process_packet(&stream_data.data, &mut tr101290_errors);
                        // Periodically, or at the end of the processing:
                        tr101290_errors.log_errors();
                    }

                     // TODO: json output here that we send into zeromq
                     debug!("STATS: PID: {}, Type: {}, Bitrate: {} bps, IAT: {} ms, Errors: {}, CC: {}, Timestamp: {} ms, Stream Type: {}", 
                                    stream_data.pid, stream_data.stream_type, stream_data.bitrate, stream_data.iat, 
                                    stream_data.error_count, stream_data.continuity_counter, stream_data.timestamp,
                                    stream_data.stream_type);

                    batch.push(stream_data.data.clone());

                    // Check if batch is full
                    if batch.len() >= batch_size {
                        // Send the batch to the channel
                        tx.send(batch.clone()).unwrap();
                        batch.clear();
                    }
                }
                
                let stats = cap.stats().unwrap();
                debug!(
                    "Received: {}, dropped: {}, if_dropped: {}",
                    stats.received, stats.dropped, stats.if_dropped
                );
            },
            Err(e) => {
                error!("Error capturing packet: {:?}", e);
                break; // or handle the error as needed
            }
        }
    }

    info!("Exiting rscap probe");

    // Send stop signal
    tx.send(Vec::new()).unwrap();
    drop(tx);

    // Wait for the zmq_thread to finish
    zmq_thread.join().unwrap();
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
const RFC_4175_EXT_SEQ_NUM_LEN: usize = 2;
const RFC_4175_HEADER_LEN: usize = 6; // Note: extended sequence number not included

fn set_extended_sequence_number(buf: &mut [u8], number: u16) {
    buf[0] = (number >> 8) as u8;
    buf[1] = number as u8;
}

fn get_extended_sequence_number(buf: &[u8]) -> u16 {
    ((buf[0] as u16) << 8) | buf[1] as u16
}

fn set_line_length(buf: &mut [u8], length: u16) {
    buf[0] = (length >> 8) as u8;
    buf[1] = length as u8;
}

fn get_line_length(buf: &[u8]) -> u16 {
    ((buf[0] as u16) << 8) | buf[1] as u16
}

fn set_line_field_id(buf: &mut [u8], id: u8) {
    buf[2] |= ((id != 0) as u8) << 7;
}

fn get_line_field_id(buf: &[u8]) -> u8 {
    buf[2] >> 7
}

fn set_line_number(buf: &mut [u8], number: u16) {
    buf[2] |= (number >> 8) as u8 & 0x7f;
    buf[3] = number as u8;
}

fn get_line_number(buf: &[u8]) -> u16 {
    ((buf[2] as u16 & 0x7f) << 8) | buf[3] as u16
}

fn set_line_continuation(buf: &mut [u8], continuation: u8) {
    buf[4] |= ((continuation != 0) as u8) << 7;
}

fn get_line_continuation(buf: &[u8]) -> u8 {
    buf[4] >> 7
}

fn set_line_offset(buf: &mut [u8], offset: u16) {
    buf[4] |= (offset >> 8) as u8 & 0x7f;
    buf[5] = offset as u8;
}

fn get_line_offset(buf: &[u8]) -> u16 {
    ((buf[4] as u16 & 0x7f) << 8) | buf[5] as u16
}
// ## End of RFC 4175 SMPTE2110 header functions ##

// Process the packet and return a vector of SMPTE ST 2110 packets
fn process_smpte2110_packet(payload_offset: usize, packet: &[u8], _packet_size: usize, start_time: u64) -> Vec<StreamData> {
    let start = payload_offset;
    let mut streams = Vec::new();

    if packet.len() > start + 12 {
        if packet[start] == 0x80 || packet[start] == 0x81 {
            let rtp_packet = &packet[start..];

            // Create an RtpReader
            if let Ok(rtp) = RtpReader::new(rtp_packet) {
                // Extract the sequence number
                let _sequence_number = rtp.sequence_number();
        
                // Extract the timestamp
                let timestamp = rtp.timestamp();

                // size of packet
                let chunk_size = rtp.payload().len();

                let payload_type = rtp.payload_type();

                let payload = rtp.payload();
               
                let line_length = get_line_length(rtp_packet);
                let line_number = get_line_number(rtp_packet);
                let line_offset = get_line_offset(rtp_packet);
                let field_id = get_line_field_id(rtp_packet);
        
                let smpte_header_info = json!({
                    "size": chunk_size,
                    //"sequence_number": sequence_number as u64,
                    "timestamp": timestamp,
                    "payload_type": payload_type,
                    "line_length": line_length,
                    "line_number": line_number,
                    "line_offset": line_offset,
                    "field_id": field_id,
                });

                let pid = 1; /* FIXME */
                let stream_type = "smpte2110".to_string();
                let mut stream_data = StreamData::new(payload, pid, stream_type, start_time, timestamp as u64, 0 /* fix me */);
                // update streams details in stream_data structure
                stream_data.update_stats(chunk_size, current_unix_timestamp_ms().unwrap_or(0));

                streams.push(stream_data);            

                //smpte2110_packets.push(rtp_packet.to_vec());
                debug!("SMPTE ST 2110 Header Info: {}", smpte_header_info.to_string());
            } else {
                hexdump(&packet);
                error!("Error parsing RTP header, not SMPTE ST 2110");
            }
        } else {
            hexdump(&packet);
            error!("No RTP header detected, not SMPTE ST 2110");
        }
    } else {
        hexdump(&packet);
        error!("Packet too small, not SMPTE ST 2110");
    }

    streams
}

// Process the packet and return a vector of MPEG-TS packets
fn process_mpegts_packet(payload_offset: usize, packet: &[u8], packet_size: usize, start_time: u64) -> Vec<StreamData> {
    //let mut mpeg_ts_packets = Vec::new();
    let mut start = payload_offset;
    let mut read_size = packet_size;
    let mut streams = Vec::new();

    while start + read_size <= packet.len() {
        let chunk = &packet[start..start + read_size];
        if chunk[0] == 0x47 { // Check for MPEG-TS sync byte
            //mpeg_ts_packets.push(chunk.to_vec());
            read_size = packet_size; // reset read_size

            let pid = ((chunk[1] as u16 & 0x1F) << 8) | chunk[2] as u16;

            // Handle PAT and PMT packets
            match pid {
                PAT_PID => {
                    log::debug!("ProcessPacket: PAT packet detected with PID {}", pid);
                    parse_and_store_pat(chunk);
                }
                _ => {
                    // Check if this is a PMT packet
                    unsafe {
                        if pid == PMT_PID {
                            log::debug!("ProcessPacket: PMT packet detected with PID {}", pid);
                            // Update PID_MAP with new stream types
                            update_pid_map(chunk);
                        }
                    }
                }
            }
    
            let stream_type = determine_stream_type(pid); // Implement this function based on PAT/PMT parsing
            let timestamp = ((chunk[4] as u64) << 25) | ((chunk[5] as u64) << 17) | ((chunk[6] as u64) << 9) | ((chunk[7] as u64) << 1) | ((chunk[8] as u64) >> 7);
            let continuity_counter = chunk[3] & 0x0F;
    
            let mut stream_data = StreamData::new(chunk, pid, stream_type, start_time, timestamp, continuity_counter);
            stream_data.update_stats(chunk.len(), current_unix_timestamp_ms().unwrap_or(0));
            streams.push(stream_data);
        } else {
            error!("ProcessPacket: Not MPEG-TS");
            hexdump(&packet);
            read_size = 1; // Skip to the next byte
        }
        start += read_size;
    }

    streams
}

// Print a hexdump of the packet
fn hexdump(packet: &[u8]) {
    let pid = extract_pid(packet);
    info!("--------------------------------------------------");
    // print in rows of 16 bytes
    info!("PacketDump: PID {} Packet length: {}", pid, packet.len());
    let mut packet_dump = String::new();
    for (i, chunk) in packet.iter().take(packet.len()).enumerate() {
        if i % 16 == 0 {
            packet_dump.push_str(&format!("\n{:04x}: ", i));
        }
        packet_dump.push_str(&format!("{:02x} ", chunk));
    }
    info!("{}", packet_dump);
    info!("");
    info!("--------------------------------------------------");
}

