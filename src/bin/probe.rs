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
//use serde_json::json;
//use log::{error, debug, info};
use tokio;
use std::net::{Ipv4Addr, UdpSocket};
use std::env;

// Able to keep up with 1080i50 422 10-bit 30 Mbps MPEG-TS stream (not long-term tested)
const BATCH_SIZE: usize = 1000; // N MPEG-TS packets per batch
const PAYLOAD_OFFSET: usize = 14 + 20 + 8; // Ethernet (14 bytes) + IP (20 bytes) + UDP (8 bytes)
const PACKET_SIZE: usize = 188; // MPEG-TS packet size
const READ_SIZE: i32 = (PACKET_SIZE as i32 * BATCH_SIZE as i32) + PAYLOAD_OFFSET as i32; // pcap read size


#[tokio::main]
async fn main() {
    // Get environment variables or use default values
    let target_port: i32 = env::var("TARGET_PORT").unwrap_or("5556".to_string()).parse().expect("Invalid format for TARGET_PORT");
    let target_ip: &str = &env::var("TARGET_IP").unwrap_or("127.0.0.1".to_string());

    let source_device: &str = &env::var("SOURCE_DEVICE").unwrap_or("en0".to_string());
    let source_device_ip: &str = &env::var("SOURCE_DEVICE_IP").unwrap_or("0.0.0.0".to_string());

    let source_port: i32 = env::var("SOURCE_PORT").unwrap_or("10000".to_string()).parse().expect("Invalid format for SOURCE_PORT");
    let source_ip: &str = &env::var("SOURCE_IP").unwrap_or("224.0.0.200".to_string());

    // Initialize logging
    env_logger::Builder::new()
    .filter_level(log::LevelFilter::Debug)
    .init();

    let context = zmq::Context::new();
    let publisher = context.socket(zmq::PUB).unwrap();
    let source_port_ip = format!("tcp://{}:{}", target_ip, target_port);
    publisher.bind(&source_port_ip).unwrap();    

    // device ip address
    let mut interface_addr = source_device_ip.parse::<Ipv4Addr>()
        .expect("Invalid IP address format in source_device_ip");

    // Get the selected device's details
    let devices = pcap::Device::list().unwrap();
    let target_device = devices.into_iter().find(|d| d.name == source_device)
        .expect("Target device not found");

    println!("{:?}", target_device);
    let mut target_device_found = false;
    for addr in target_device.addresses.iter() {
        if let std::net::IpAddr::V4(ipv4_addr) = addr.addr {
            println!("ipv4_addr: {:?}", ipv4_addr);
            interface_addr = ipv4_addr;
            target_device_found = true;
            break;
        }
    }

    // If the device is not found, search for it
    if !target_device_found {
        println!("Target device {} not found, searching...", source_device);
        for device in pcap::Device::list().unwrap() {
            println!("{:?}", device);
            
            for addr in device.addresses.iter() {
                if let std::net::IpAddr::V4(ipv4_addr) = addr.addr {
                    // check if localhost
                    if ipv4_addr.is_loopback() {
                        continue;
                    }
                    println!("ipv4_addr: {:?}", ipv4_addr);
                    target_device_found = true;

                    println!("Found IPv4 target device {} with ip {}", source_device, ipv4_addr);
                    interface_addr = ipv4_addr;
                    break;
                }
            }
            if target_device_found {
                break;
            }
            
        }
    }
    if !target_device_found {
        println!("Target device {} not found", source_device);
        return;
    }

    let multicast_addr = source_ip.parse::<Ipv4Addr>()
        .expect("Invalid IP address format in source_ip");

    let socket = UdpSocket::bind("0.0.0.0:0").expect("Failed to bind socket");
    socket.join_multicast_v4(&multicast_addr, &interface_addr)
    .expect("Failed to join multicast group");

    // Setup packet capture
    let mut cap = Capture::from_device(target_device).unwrap()
        .promisc(false)
        .snaplen(READ_SIZE) // Adjust this based on network configuration
        .open().unwrap();

    // Filter pcap
    let source_host_and_port = format!("udp dst port {} and ip dst host {}", source_port, source_ip);
    cap.filter(&source_host_and_port, true).unwrap();

    let mut total_bytes = 0;
    let mut count = 0;
    let mut batch = Vec::new();
    while let Ok(packet) = cap.next_packet() { 
        println!("received packet! {:?}", packet.header);
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

