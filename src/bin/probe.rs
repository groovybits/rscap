/*
 * rscap: probe - Rust Stream Capture with pcap, output to ZeroMQ
 *
 * Written in 2023 by Chris Kennedy (C) LTN Global
 *
 * License: LGPL v2.1
 *
 */

extern crate zmq;
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


// Able to keep up with 1080p60 420/422 8/10-bit 20+ Mbps MPEG-TS stream (not long-term tested)
const BATCH_SIZE: usize = 1000; // N MPEG-TS packets per batch
const PAYLOAD_OFFSET: usize = 14 + 20 + 8; // Ethernet (14 bytes) + IP (20 bytes) + UDP (8 bytes)
const PACKET_SIZE: usize = 188; // MPEG-TS packet size
const READ_SIZE: i32 = (PACKET_SIZE as i32 * BATCH_SIZE as i32) + PAYLOAD_OFFSET as i32; // pcap read size

// constant for PAT PID
const PAT_PID: u16 = 0;

// global variable to store PMT PID (initially set to an invalid PID)
static mut PMT_PID: u16 = 0xFFFF;

static mut LAST_PAT_PACKET: Option<Vec<u8>> = None;

lazy_static! {
    static ref PID_MAP: Mutex<HashMap<u16, String>> = Mutex::new(HashMap::new());
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
struct StreamData {
    pid: u16,
    stream_type: String, // "video", "audio", "text"
    continuity_counter: u8,
    timestamp: u64,
    data: Vec<u8>, // The actual MPEG-TS packet data
}

// StreamData implementation
impl StreamData {
    fn new(packet: &[u8], pid: u16, stream_type: String, timestamp: u64, continuity_counter: u8) -> Self {        
        StreamData {
            pid,
            stream_type,
            continuity_counter,
            timestamp,
            data: packet.to_vec(),
        }
    }
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

        debug!("ParsePAT: Packet1 {}, Packet2 {}, Packet3 {}, Packet4 {}", packet[i], packet[i + 1], packet[i + 2], packet[i + 3]);

        if program_number != 0 && program_number != 65535 && pmt_pid != 0 && program_number < 30 /* FIXME: kludge fix for now */ {
            info!("ParsePAT: Program Number: {} PMT PID: {}", program_number, pmt_pid);
            entries.push(PatEntry { program_number, pmt_pid });
        }

        i += 4;
    }

    entries
}

fn parse_pmt(packet: &[u8], pmt_pid: u16) -> Pmt {
    // Skipping error checking and assuming payload starts at a fixed position
    let program_number = ((packet[8] as u16) << 8) | (packet[9] as u16);
    let mut entries = Vec::new();
    let mut i = 17 + packet[15] as usize; // Starting index of the first stream in the PMT

    info!("ParsePMT: Program Number: {} PMT PID: {}", program_number, pmt_pid);
    while i + 5 <= packet.len() {
        let stream_type = packet[i];
        let stream_pid = (((packet[i + 1] as u16) & 0x1F) << 8) | (packet[i + 2] as u16);
        entries.push(PmtEntry { stream_pid, stream_type });
        info!("ParsePMT: Stream PID: {}, Stream Type: {}", stream_pid, stream_type);

        i += 5 + ((packet[i + 3] as usize) << 8 | packet[i + 4] as usize);
    }

    Pmt {
        entries,
    }
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
        info!("UpdatePIDmap: Processing Program Number: {}, PMT PID: {}", program_number, pmt_pid);

        // Ensure the current PMT packet matches the PMT PID from the PAT
        if extract_pid(pmt_packet) == pmt_pid {
            let pmt = parse_pmt(pmt_packet, pmt_pid);

            for pmt_entry in pmt.entries.iter() {
                let stream_pid = pmt_entry.stream_pid;
                let stream_type = match pmt_entry.stream_type {
                    0x02 => "MPEG-2 Video",
                    0x03 => "MPEG-1 Audio",
                    0x04 => "MPEG-2 Audio",
                    0x0f => "AAC Audio",
                    0x1b => "H.264 Video",
                    0x24 => "HEVC Video",
                    0x80 => "Dolby Digital (AC-3) Audio",
                    0x81 => "Dolby Digital Plus (E-AC-3) Audio",
                    // ... other types ...
                    _ => "Unknown",
                };

                info!("UpdatePIDmap: Added Stream PID: {}, Stream Type: {}", stream_pid, stream_type);
                pid_map.insert(stream_pid, stream_type.to_string());
            }
        } else {
            info!("UpdatePIDmap: Skipping PMT PID: {} as it does not match with current PMT packet PID", pmt_pid);
        }
    }
}

fn determine_stream_type(pid: u16) -> String {
    let pid_map = PID_MAP.lock().unwrap();
    pid_map.get(&pid).cloned().unwrap_or_else(|| "unknown".to_string())
}

// MAIN
#[tokio::main]
async fn main() {
    info!("Starting rscap probe");
    dotenv::dotenv().ok(); // read .env file

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
        .timeout(60000)
        .snaplen(READ_SIZE) // Adjust this based on network configuration
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

    // Start packet capture
    let mut batch = Vec::new();
    loop {
        match cap.next_packet() {
            Ok(packet) => {
                if debug_on{
                    debug!("Received packet! {:?}", packet.header);
                }

                // Check if chunk is MPEG-TS or SMPTE 2110
                let chunk_type = is_mpegts_or_smpte2110(&packet[PAYLOAD_OFFSET..]);
                if chunk_type != 1 {
                    debug!("Not MPEG-TS, type {}", chunk_type);
                    if chunk_type == 0 {
                        error!("Not MPEG-TS or SMPTE 2110");
                        hexdump(&packet);
                    }
                    is_mpegts = false;
                }

                let chunks = if is_mpegts {
                    process_mpegts_packet(&packet)
                } else {
                    process_smpte2110_packet(&packet)
                };

                // Process each chunk
                for stream_data in chunks {
                    if debug_on {
                        hexdump(&stream_data.data); // Use stream_data.data to access the raw packet data
                    }

                    // print out the stream data parts outside of the .data
                    debug!("PID: {}, Stream Type: {}, Continuity Counter: {}, Timestamp: {}", stream_data.pid, stream_data.stream_type, stream_data.continuity_counter, stream_data.timestamp);

                    batch.push(stream_data.data.clone());

                    // Check if batch is full
                    if batch.len() >= BATCH_SIZE {
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

// Process the packet and return a vector of SMPTE ST 2110 packets
fn process_smpte2110_packet(packet: &[u8]) -> Vec<StreamData> {
    let mut smpte2110_packets = Vec::new();
    let start = PAYLOAD_OFFSET;

    if packet.len() > start + 12 {
        if packet[start] == 0x80 || packet[start] == 0x81 {
            let rtp_packet = &packet[start..];
            let smpte_header_info = json!({
                "size": rtp_packet.len(),
            });
            smpte2110_packets.push(rtp_packet.to_vec());
            debug!("SMPTE ST 2110 Header Info: {}", smpte_header_info.to_string());
        } else {
            error!("Not RTP");
            hexdump(&packet);
        }
    }

    let mut streams = Vec::new();

    for chunk in smpte2110_packets.iter() {
        let pid = 1;
        let stream_type = "smpte2110".to_string();

        // get current time
        match current_unix_timestamp_ms() {
            Ok(timestamp) => {
                let stream_data = StreamData::new(chunk, pid, stream_type, timestamp, 1);
                streams.push(stream_data);            
            },
            Err(e) => error!("Error: {}", e),
        }
    }

    streams
}

// Process the packet and return a vector of MPEG-TS packets
fn process_mpegts_packet(packet: &[u8]) -> Vec<StreamData> {
    let mut mpeg_ts_packets = Vec::new();
    let mut start = PAYLOAD_OFFSET;
    let mut read_size = PACKET_SIZE;

    while start + read_size <= packet.len() {
        let chunk = &packet[start..start + read_size];
        if chunk[0] == 0x47 { // Check for MPEG-TS sync byte
            mpeg_ts_packets.push(chunk.to_vec());
            read_size = PACKET_SIZE; // reset read_size
        } else {
            error!("ProcessPacket: Not MPEG-TS");
            hexdump(&packet);
            read_size = 1; // Skip to the next byte
        }
        start += read_size;
    }

    let mut streams = Vec::new();

    for chunk in mpeg_ts_packets.iter() {
        let pid = ((chunk[1] as u16 & 0x1F) << 8) | chunk[2] as u16;

        // Check for PAT packet
        if pid == PAT_PID {
            log::info!("ProcessPacket: PAT packet detected with pid {}", pid);
            let pat_entries = parse_pat(chunk); // Use 'chunk' instead of 'packet'
            unsafe {
                // Assuming there's only one program for simplicity
                PMT_PID = pat_entries.first().map_or(0xFFFF, |entry| entry.pmt_pid);
                log::info!("ProcessPacket: PMT PID: {}", PMT_PID);
                LAST_PAT_PACKET = Some(chunk.to_vec()); // Store the specific PAT chunk
            }
        } else {
            log::info!("ProcessPacket: PID: {}", pid);
        }

        // Check for PMT packet
        unsafe {
            // convert PMT_PID to u16
            if pid == PMT_PID {
                log::info!("ProcessPacket: PMT packet detected with pid {}/{}", pid, PMT_PID);
                // Update PID_MAP with new stream types
                update_pid_map(chunk);
            }
        }

        let stream_type = determine_stream_type(pid); // Implement this function based on PAT/PMT parsing
        let timestamp = ((chunk[4] as u64) << 25) | ((chunk[5] as u64) << 17) | ((chunk[6] as u64) << 9) | ((chunk[7] as u64) << 1) | ((chunk[8] as u64) >> 7);
        let continuity_counter = chunk[3] & 0x0F;

        let stream_data = StreamData::new(chunk, pid, stream_type, timestamp, continuity_counter);
        streams.push(stream_data);
    }

    streams
}

// Print a hexdump of the packet
fn hexdump(packet: &[u8]) {
    let pid = extract_pid(packet);
    debug!("--------------------------------------------------");
    // print in rows of 16 bytes
    debug!("PacketDump: PID {} Packet length: {}", pid, packet.len());
    let mut packet_dump = String::new();
    for (i, chunk) in packet.iter().take(packet.len()).enumerate() {
        if i % 16 == 0 {
            packet_dump.push_str(&format!("\n{:04x}: ", i));
        }
        packet_dump.push_str(&format!("{:02x} ", chunk));
    }
    debug!("{}", packet_dump);
    debug!("");
    debug!("--------------------------------------------------");
}

