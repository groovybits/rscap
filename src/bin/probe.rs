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
    pmt_pid: u16,
    program_number: u16,
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
    fn new(packet: &[u8], pid: u16, stream_type: String) -> Self {
        let continuity_counter = packet[3] & 0x0F;
        let timestamp = ((packet[4] as u64) << 25) | ((packet[5] as u64) << 17) | ((packet[6] as u64) << 9) | ((packet[7] as u64) << 1) | ((packet[8] as u64) >> 7);
        StreamData {
            pid,
            stream_type,
            continuity_counter,
            timestamp,
            data: packet.to_vec(),
        }
    }
}

fn parse_pat(packet: &[u8]) -> Vec<PatEntry> {
    // Skipping error checking and assuming payload starts at a fixed position
    let mut entries = Vec::new();
    let mut i = 8; // Starting index of the first program in the PAT

    while i + 4 <= packet.len() {
        let program_number = ((packet[i] as u16) << 8) | (packet[i + 1] as u16);
        let pmt_pid = (((packet[i + 2] as u16) & 0x1F) << 8) | (packet[i + 3] as u16);

        if program_number != 0 {
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

    while i + 5 <= packet.len() {
        let stream_type = packet[i];
        let stream_pid = (((packet[i + 1] as u16) & 0x1F) << 8) | (packet[i + 2] as u16);
        entries.push(PmtEntry { stream_pid, stream_type });

        i += 5 + ((packet[i + 3] as usize) << 8 | packet[i + 4] as usize);
    }

    Pmt {
        pmt_pid,
        program_number,
        entries,
    }
}

// Modify the function to use the stored PAT packet
fn update_pid_map_from_pat_pmt(pmt_packet: &[u8]) {
    let mut pid_map = PID_MAP.lock().unwrap();

    unsafe {
        if let Some(pat_packet) = &LAST_PAT_PACKET {
            let program_pids = parse_pat(pat_packet);

            for pat_entry in program_pids.iter() {
                let program_number = pat_entry.program_number;
                let stream_types = parse_pmt(pmt_packet, program_number);

                for pmt_entry in stream_types.entries.iter() {
                    let stream_pid = pmt_entry.stream_pid;
                    let stream_type = pmt_entry.stream_type;
                    pid_map.insert(stream_pid, stream_type.to_string());
                }
            }
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
        println!("Device: {:?}, Flags: {:?}", device.name, device.flags);
    }
    #[cfg(target_os = "linux")]
    let mut target_device = devices.clone().into_iter()
        .find(|d| d.name != "lo") // Exclude loopback device
        .expect("No valid devices found");

    #[cfg(not(target_os = "linux"))]
    let mut target_device = devices.clone().into_iter()
        .find(|d| d.flags.is_up() && !d.flags.is_loopback() && d.flags.is_running() && (!d.flags.is_wireless() || use_wireless))
        .expect(&format!("No valid devices found {}", devices.len()));

    println!("Default device: {:?}", target_device.name);

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

                    // Check if chunk is MPEG-TS or SMPTE 2110
                    let chunk_type = is_mpegts_or_smpte2110(&stream_data.data);

                    if chunk_type == 1 || chunk_type == 2 {
                        batch.push(stream_data.data.clone());
                        if chunk_type == 2 {
                            debug!("SMPTE 2110 packet detected");
                            is_mpegts = false;
                        }

                        // Check if batch is full
                        if batch.len() >= BATCH_SIZE {
                            // Send the batch to the channel
                            tx.send(batch.clone()).unwrap();
                            batch.clear();
                        }
                    } else {
                        hexdump(&stream_data.data);
                        error!("Not MPEG-TS or SMPTE 2110");
                    }
                }
                let stats = cap.stats().unwrap();
                println!(
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

// Implement a function to extract PID from a packet
fn extract_pid(packet: &[u8]) -> u16 {
    // Extract PID from packet
    // (You'll need to adjust the indices according to your packet format)
    ((packet[1] as u16 & 0x1F) << 8) | packet[2] as u16
}

// Update the update_pid_map_from_pat_pmt function to handle PMT data
fn update_pid_map_from_pmt(pmt: Pmt) {
    let mut pid_map = PID_MAP.lock().unwrap();
    for pmt_entry in pmt.entries.iter() {
        let stream_pid = pmt_entry.stream_pid;
        // Convert stream_type to a human-readable format
        let stream_type = match pmt_entry.stream_type {
            // Add cases for different stream types
            0x02 => "MPEG Video",
            // ... other types ...
            _ => "Unknown",
        };
        pid_map.insert(stream_pid, stream_type.to_string());
    }
}

// Process the packet and return a vector of SMPTE ST 2110 packets
fn process_smpte2110_packet(packet: &[u8]) -> Vec<StreamData> {
    let mut smpte2110_packets = Vec::new();
    let start = PAYLOAD_OFFSET;

    if packet.len() > start + 12 {
        if packet[start] == 0x80 || packet[start] == 0x81 {
            let rtp_packet = &packet[start..];
            // Create StreamData for each SMPTE 2110 packet
            let stream_data = StreamData {
                pid: 0, // Placeholder PID
                stream_type: "smpte2110".to_string(), // Placeholder stream type
                continuity_counter: 0, // Placeholder continuity counter
                timestamp: 0, // Placeholder timestamp
                data: rtp_packet.to_vec(),
            };
            smpte2110_packets.push(stream_data);
        }
    }

    smpte2110_packets
}

// Process the packet and return a vector of MPEG-TS packets
fn process_mpegts_packet(packet: &[u8]) -> Vec<StreamData> {
    let mut mpeg_ts_packets = Vec::new();
    let mut start = PAYLOAD_OFFSET;

    while start + PACKET_SIZE <= packet.len() {
        let chunk = &packet[start..start + PACKET_SIZE];
        if chunk[0] == 0x47 { // Check for MPEG-TS sync byte
            mpeg_ts_packets.push(chunk.to_vec());

            // Extract the PID from the MPEG-TS header
            let pid = extract_pid(chunk); //((chunk[1] as u16 & 0x1F) << 8) | chunk[2] as u16;

            // Check for PAT packet
            if pid == PAT_PID {
                log::info!("PAT packet detected with pid {}", pid);
                let pat_entries = parse_pat(&packet);
                unsafe {
                    // Assuming there's only one program for simplicity
                    PMT_PID = pat_entries.first().map_or(0xFFFF, |entry| entry.pmt_pid);
                    log::info!("PMT PID: {}", PMT_PID);
                    LAST_PAT_PACKET = Some(packet[start..start + PACKET_SIZE].to_vec());
                }
            }

            // Check for PMT packet
            unsafe {
                if pid == PMT_PID {
                    log::info!("PMT packet detected with pid {}", pid);
                    let pmt = parse_pmt(&packet, PMT_PID);
                    // Update PID_MAP with new stream types
                    update_pid_map_from_pmt(pmt);

                    update_pid_map_from_pat_pmt(&packet);
                }
            }

            // Extract the continuity counter from the MPEG-TS header
            let continuity_counter = chunk[3] & 0x0F;

            // Extract the adaptation field control from the MPEG-TS header
            let adaptation_field_control = (chunk[3] & 0x30) >> 4;

            // Extract the payload unit start indicator from the MPEG-TS header
            let payload_unit_start_indicator = chunk[1] & 0x40;

            // Extract the transport scrambling control from the MPEG-TS header
            let transport_scrambling_control = (chunk[3] & 0xC0) >> 6;

            // Extract the transport error indicator from the MPEG-TS header
            let transport_error_indicator = chunk[1] & 0x80;

            // Extract the transport priority from the MPEG-TS header
            let transport_priority = chunk[2] & 0x20;

            // Extract the transport private data from the MPEG-TS header
            let transport_private_data = chunk[2] & 0x80;

            // extract the timestamp from the MPEG-TS header
            let timestamp = ((chunk[4] as u64) << 25) | ((chunk[5] as u64) << 17) | ((chunk[6] as u64) << 9) | ((chunk[7] as u64) << 1) | ((chunk[8] as u64) >> 7);

            // Construct JSON object with MPEG-TS header information
            let mpegts_header_info = json!({
                "pid": pid,
                "continuity_counter": continuity_counter,
                "adaptation_field_control": adaptation_field_control,
                "payload_unit_start_indicator": payload_unit_start_indicator,
                "transport_scrambling_control": transport_scrambling_control,
                "transport_error_indicator": transport_error_indicator,
                "transport_priority": transport_priority,
                "transport_private_data": transport_private_data,
                "timestamp": timestamp,
            });

            // Print out the JSON structure
            debug!("MPEG-TS Header Info: {}", mpegts_header_info.to_string());
        }
        start += PACKET_SIZE;
    }

    let mut streams = Vec::new();

    for chunk in mpeg_ts_packets.iter() {
        let pid = ((chunk[1] as u16 & 0x1F) << 8) | chunk[2] as u16;
        let stream_type = determine_stream_type(pid); // Implement this function based on PAT/PMT parsing

        let stream_data = StreamData::new(chunk, pid, stream_type);
        streams.push(stream_data);
    }

    streams
}

// Print a hexdump of the packet
fn hexdump(packet: &[u8]) {
    // print in rows of 16 bytes
    println!("Packet length: {}", packet.len());
    for (i, chunk) in packet.iter().take(PACKET_SIZE).enumerate() {
        if i % 16 == 0 {
            print!("\n{:04x}: ", i);
        }
        print!("{:02x} ", chunk);
    }
    println!("");
    println!("--------------------------------------------------");
}

