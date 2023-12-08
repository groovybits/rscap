/*
 * client.rs
 *
 * This is a part of a simple ZeroMQ-based MPEG-TS capture and playback system.
 * This file contains the client-side code that receives MPEG-TS chunks from the
 * server and writes them to a file.
 *
 * Author: Chris Kennedy (C) 2023 LTN Global
 *
 * License: LGPL v2.1
 *
 */

extern crate zmq;
use log::{error};
use tokio;
use std::fs::File;
use std::io::Write;
//use ffmpeg_next as ffmpeg; // You might need to configure FFmpeg flags

const SOURCE_PORT: i32 = 5556; // TODO: change to your target port
const SOURCE_IP: &str = "127.0.0.1";
const OUTPUT_FILE: &str = "output.ts";

#[tokio::main]
async fn main() {
    env_logger::Builder::new()
    .filter_level(log::LevelFilter::Debug)
    .init();

    // Setup ZeroMQ subscriber
    let context = zmq::Context::new();
    let zmq_sub = context.socket(zmq::SUB).unwrap();
    let source_port_ip = format!("tcp://{}:{}", SOURCE_IP, SOURCE_PORT);
    if let Err(e) = zmq_sub.connect(&source_port_ip) {
        error!("Failed to connect ZeroMQ subscriber: {:?}", e);
        return;
    }

    zmq_sub.set_subscribe(b"").unwrap();

    let mut file = File::create(OUTPUT_FILE).unwrap();

    let mut total_bytes = 0;
    let mut count = 0;
    while let Ok(msg) = zmq_sub.recv_bytes(0) {
        total_bytes += msg.len();
        count += 1;
        println!("#{} Received {}/{} bytes", count, msg.len(), total_bytes);
        // write to file, appending if not first chunk
        file.write_all(&msg).unwrap();
    }

    // Now check the file with FFmpeg for MPEG-TS integrity
    // ... FFmpeg checking logic goes here ...
    //check_mpegts_integrity("output.ts");
}

/*
fn check_mpegts_integrity(file_path: &str) {
    // Initialize FFmpeg
    ffmpeg::init().unwrap();

    // Open the file with FFmpeg
    let mut format_context = ffmpeg::format::input(&file_path).unwrap();

    // Iterate over the packets and check MPEG-TS headers and continuity counters
    // ... Detailed FFmpeg packet checking logic goes here ...
    while let Some(packet) = format_context.packets().next() {
        let packet = packet.unwrap();
        // Check MPEG-TS header
        let mpegts_header = packet.data();
        println!("MPEG-TS header: {:?}", mpegts_header);
        // Check continuity counter
        let continuity_counter = mpegts_header[3] & 0x0F;
        println!("Continuity counter: {}", continuity_counter);
    }
}
*/

