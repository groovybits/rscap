extern crate zmq;
use pcap::{Capture, Device};
//use serde_json::json;
//use log::{error, debug, info};
use tokio;

const BATCH_SIZE: usize = 10; // Number of MPEG-TS packets per batch

#[tokio::main]
async fn main() {
    // Initialize logging
    env_logger::Builder::new()
    .filter_level(log::LevelFilter::Debug)
    .init();

    let context = zmq::Context::new();
    let publisher = context.socket(zmq::PUB).unwrap();
    publisher.bind("tcp://127.0.0.1:5556").unwrap();

    // List available devices
    let default_device = Device::lookup().unwrap();
    println!("Default device: {:?}", default_device);
    let target_device = "en7";
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
        .snaplen((188 + 8 + 20 + 14) * 1000) // Adjust this based on network configuration
        .open().unwrap();

    // Filter for UDP port 10000
    cap.filter("udp dst port 10000", true).unwrap();

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
    // identifying MPEG-TS or SMPTE 2110

    // MPEG-TS typically starts with a 0x47 sync byte.
    return packet.starts_with(&[0x47]) || // Simplified example for MPEG-TS

    // RTP header identification for SMPTE 2110-20
    packet.starts_with(&[0x80]); // Simplified example for RTP
}

fn process_packet(packet: &[u8]) -> Vec<Vec<u8>> {
    // Strip off network headers to get to the MPEG-TS payload
    // The exact amount to strip depends on network configuration
    // Default is Ethernet (14 bytes) + IP (20 bytes) + UDP (8 bytes)
    let payload_offset = 14 + 20 + 8; // Adjust this based on network configuration

    let mut mpeg_ts_packets = Vec::new();
    let mut start = payload_offset;

    while start + 188 <= packet.len() {
        let chunk = &packet[start..start + 188];
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
