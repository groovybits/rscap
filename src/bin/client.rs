extern crate zmq;
use log::{error};
use tokio;
use std::fs::File;
use std::io::Write;
//use ffmpeg_next as ffmpeg; // You might need to configure FFmpeg flags

#[tokio::main]
async fn main() {
    env_logger::Builder::new()
    .filter_level(log::LevelFilter::Debug)
    .init();

    // Setup ZeroMQ subscriber
    let context = zmq::Context::new();
    let zmq_sub = context.socket(zmq::SUB).unwrap();
    if let Err(e) = zmq_sub.connect("tcp://127.0.0.1:5556") {
        error!("Failed to connect ZeroMQ subscriber: {:?}", e);
        return;
    }

    zmq_sub.set_subscribe(b"").unwrap();

    let mut file = File::create("output.ts").unwrap();

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

