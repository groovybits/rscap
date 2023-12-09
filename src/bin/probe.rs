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

// Able to keep up with 1080p60 420/422 8/10-bit 20+ Mbps MPEG-TS stream (not long-term tested)
const BATCH_SIZE: usize = 1000; // N MPEG-TS packets per batch
const PAYLOAD_OFFSET: usize = 14 + 20 + 8; // Ethernet (14 bytes) + IP (20 bytes) + UDP (8 bytes)
const PACKET_SIZE: usize = 188; // MPEG-TS packet size
const READ_SIZE: i32 = (PACKET_SIZE as i32 * BATCH_SIZE as i32) + PAYLOAD_OFFSET as i32; // pcap read size

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
    let source_host_and_port = format!("udp dst port {} and ip dst host {}", source_port, source_ip);
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
                for chunk in chunks {
                    if debug_on {
                        hexdump(&chunk);
                    }

                    // Check if chunk is MPEG-TS or SMPTE 2110
                    let chunk_type = is_mpegts_or_smpte2110(&chunk);
                    if chunk_type == 1 || chunk_type == 2 {
                        batch.push(chunk);
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
                        hexdump(&chunk);
                        error!("Not MPEG-TS or SMPTE 2110");
                    }
                }
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
fn process_smpte2110_packet(packet: &[u8]) -> Vec<Vec<u8>> {
    let mut smpte2110_packets = Vec::new();
    let start = PAYLOAD_OFFSET;

    if packet.len() > start + 12 {
        if packet[start] == 0x80 || packet[start] == 0x81 {
            let rtp_packet = &packet[start..];
            smpte2110_packets.push(rtp_packet.to_vec());

            // Extract RTP header information
            let sequence_number = u16::from_be_bytes([packet[start + 2], packet[start + 3]]);
            let timestamp = u32::from_be_bytes([packet[start + 4], packet[start + 5], packet[start + 6], packet[start + 7]]);
            let ssrc = u32::from_be_bytes([packet[start + 8], packet[start + 9], packet[start + 10], packet[start + 11]]);

            // Extract the payload type from the RTP header
            let payload_type = packet[start + 1] & 0x7F;

            // Extract the marker bit from the RTP header
            let marker_bit = (packet[start + 1] & 0x80) >> 7;

            // Check if interlaced video
            let interlaced = (packet[start + 12] & 0x80) >> 7;

            // Check if UHD
            let uhd = (packet[start + 12] & 0x40) >> 6;

            // Check if 10-bit
            let ten_bit = (packet[start + 12] & 0x20) >> 5;

            // Check if 422
            let four_two_two = (packet[start + 12] & 0x10) >> 4;

            // Check if 444
            let four_four_four = (packet[start + 12] & 0x08) >> 3;

            // Construct JSON object with RTP header information
            let rtp_header_info = json!({
                "sequence_number": sequence_number,
                "timestamp": timestamp,
                "ssrc": ssrc,
                "payload_type": payload_type,
                "marker_bit": marker_bit,
                "interlaced": interlaced,
                "uhd": uhd,
                "ten_bit": ten_bit,
                "four_two_two": four_two_two,
                "four_four_four": four_four_four
            });

            // Print out the JSON structure
            debug!("RTP Header Info: {}", rtp_header_info.to_string());
        }
    }

    smpte2110_packets
}


// Process the packet and return a vector of MPEG-TS packets
fn process_mpegts_packet(packet: &[u8]) -> Vec<Vec<u8>> {
    let mut mpeg_ts_packets = Vec::new();
    let mut start = PAYLOAD_OFFSET;

    while start + PACKET_SIZE <= packet.len() {
        let chunk = &packet[start..start + PACKET_SIZE];
        if chunk[0] == 0x47 { // Check for MPEG-TS sync byte
            mpeg_ts_packets.push(chunk.to_vec());

            // Extract the PID from the MPEG-TS header
            let pid = ((chunk[1] as u16 & 0x1F) << 8) | chunk[2] as u16;

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

            /*
            // extrace the interlaced status from the MPEG-TS header
            let interlaced = chunk[8] & 0x40;

            // extract the uhd status from the MPEG-TS header
            let uhd = chunk[8] & 0x20;

            // extract the 10-bit status from the MPEG-TS header
            let ten_bit = chunk[8] & 0x10;

            // extract the 422 status from the MPEG-TS header
            let four_two_two = chunk[8] & 0x08;

            // extract the 444 status from the MPEG-TS header
            let four_four_four = chunk[8] & 0x04;
            */

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
                /*"interlaced": interlaced,
                "uhd": uhd,
                "ten_bit": ten_bit,
                "four_two_two": four_two_two,
                "four_four_four": four_four_four*/
            });

            // Print out the JSON structure
            debug!("MPEG-TS Header Info: {}", mpegts_header_info.to_string());
        }
        start += PACKET_SIZE;
    }

    mpeg_ts_packets
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

