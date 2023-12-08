/*
 * rscap: probe - Rust Stream Capture with pcap, output to ZeroMQ
 *
 * Written in 2023 by Chris Kennedy (C) LTN Global
 *
 * License: LGPL v2.1
 *
 */

extern crate zmq;
use pcap::{Capture, Device};
//use serde_json::json;
//use log::{error, debug, info};
use tokio;

// Able to keep up with 1080i50 422 10-bit 30 Mbps MPEG-TS stream (not long-term tested)
const BATCH_SIZE: usize = 1000; // N MPEG-TS packets per batch
const PAYLOAD_OFFSET: usize = 14 + 20 + 8; // Ethernet (14 bytes) + IP (20 bytes) + UDP (8 bytes)
const PACKET_SIZE: usize = 188; // MPEG-TS packet size
const READ_SIZE: i32 = (PACKET_SIZE as i32 * BATCH_SIZE as i32) + PAYLOAD_OFFSET as i32; // pcap read size

// TODO: change to your target address and port
const TARGET_PORT: i32 = 5556; // TODO: change to your target port
const TARGET_IP: &str = "127.0.0.1"; // TODO: change to your target IP

// TODO: change to your source device
const SOURCE_DEVICE: &str = "en0"; // TODO: change to your network interface

// TODO: change to your source address and port
const SOURCE_PORT: i32 = 10000; // TODO: change to your source port
const SOURCE_IP: &str = "224.0.0.200"; // TODO: change to your source IP


#[tokio::main]
async fn main() {
    // Initialize logging
    env_logger::Builder::new()
    .filter_level(log::LevelFilter::Debug)
    .init();

    let context = zmq::Context::new();
    let publisher = context.socket(zmq::PUB).unwrap();
    let source_port_ip = format!("tcp://{}:{}", TARGET_IP, TARGET_PORT);
    publisher.bind(&source_port_ip).unwrap();    

    // List available devices
    let default_device = Device::lookup().unwrap();
    println!("Default device: {:?}", default_device);
    let target_device = SOURCE_DEVICE;
    let mut target_device_found = false;
    for device in pcap::Device::list().unwrap() {
        println!("{:?}", device);
        if device.name == target_device {
            println!("Found target device: {}", target_device);
            target_device_found = true;
        }
    }
    // Exit if target device not found
    if !target_device_found {
        println!("Target device not found: {}", target_device);
        return;
    }

    // Setup packet capture
    let mut cap = Capture::from_device(target_device).unwrap()
        .promisc(false)
        .snaplen(READ_SIZE) // Adjust this based on network configuration
        .open().unwrap();

    // Filter pcap
    let source_host_and_port = format!("udp dst port {} and ip dst host {}", SOURCE_PORT, SOURCE_IP);
    cap.filter(&source_host_and_port, true).unwrap();

    let mut total_bytes = 0;
    let mut count = 0;
    let mut batch = Vec::new();
    while let Ok(packet) = cap.next() { 
        let chunks = process_packet(&packet);

        for chunk in chunks {
            if is_mpegts_or_smpte2110(&chunk) {
                //hexdump(&chunk);
                //println!("--------------------------------------------------");
                batch.push(chunk);

                if batch.len() >= BATCH_SIZE {
                    let batched_data = batch.concat();

                    // Construct JSON header for batched data
                    /*let json_header = json!({
                        "type": "mpegts_chunk",
                        "content_length": batched_data.len(),
                    });

                    // Send JSON header as multipart message
                    publisher.send(json_header.to_string().as_bytes(), zmq::SNDMORE).unwrap();
                    */

                    // Send chunk
                    let chunk_size = batched_data.len();
                    total_bytes += chunk_size;
                    count += 1;
                    publisher.send(batched_data, 0).unwrap();
                    
                    println!("#{} Sent chunk of {}/{} bytes", count, chunk_size, total_bytes);

                    batch.clear();
                }
            } else {
                hexdump(&chunk);
                println!("Not MPEG-TS or SMPTE 2110");
                println!("--------------------------------------------------");
            }
        }
        
    }
}

fn is_mpegts_or_smpte2110(packet: &[u8]) -> bool {
    // identifying MPEG-TS, TODO: check for SMPTE 2110

    // MPEG-TS typically starts with a 0x47 sync byte.
    return packet.starts_with(&[0x47]);
}

fn process_packet(packet: &[u8]) -> Vec<Vec<u8>> {
    // Strip off network headers to get to the MPEG-TS payload
    // The exact amount to strip depends on network configuration
    // Default is Ethernet (14 bytes) + IP (20 bytes) + UDP (8 bytes)

    let mut mpeg_ts_packets = Vec::new();
    let mut start = PAYLOAD_OFFSET;

    while start + 188 <= packet.len() {
        let chunk = &packet[start..start + PACKET_SIZE];
        if chunk[0] == 0x47 { // Check for MPEG-TS sync byte
            mpeg_ts_packets.push(chunk.to_vec());
        }
        start += 188;
    }

    mpeg_ts_packets
}

fn hexdump(packet: &[u8]) {
    // print in rows of 16 bytes
    for (i, chunk) in packet.iter().take(188).enumerate() {
        if i % 16 == 0 {
            print!("\n{:04x}: ", i);
        }
        print!("{:02x} ", chunk);
    }
    println!();
}
