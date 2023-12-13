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
use log::{debug, error, info};
use std::env;
use std::fs::File;
use std::io::Write;
use tokio;
//use ffmpeg_next as ffmpeg; // You might need to configure FFmpeg flags

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok(); // read .env file

    // Get environment variables or use default values, set in .env file
    let source_port: i32 = env::var("TARGET_PORT")
        .unwrap_or("5556".to_string())
        .parse()
        .expect(&format!("Invalid format for TARGET_PORT"));
    let source_ip: &str = &env::var("TARGET_IP").unwrap_or("127.0.0.1".to_string());
    let debug_on: bool = env::var("DEBUG")
        .unwrap_or("false".to_string())
        .parse()
        .expect(&format!("Invalid format for DEBUG"));
    let silent: bool = env::var("SILENT")
        .unwrap_or("false".to_string())
        .parse()
        .expect(&format!("Invalid format for SILENT"));
    let output_file: &str = &env::var("OUTPUT_FILE").unwrap_or("output.ts".to_string());
    let send_json_header: bool = env::var("SEND_JSON_HEADER")
        .unwrap_or("false".to_string())
        .parse()
        .expect(&format!("Invalid format for SEND_JSON_HEADER"));

    info!("Starting rscap client");

    if !silent {
        let mut log_level: log::LevelFilter = log::LevelFilter::Info;
        if debug_on {
            log_level = log::LevelFilter::Debug;
        }
        env_logger::Builder::new().filter_level(log_level).init();
    }

    // Setup ZeroMQ subscriber
    let context = zmq::Context::new();
    let zmq_sub = context.socket(zmq::SUB).unwrap();
    let source_port_ip = format!("tcp://{}:{}", source_ip, source_port);
    if let Err(e) = zmq_sub.connect(&source_port_ip) {
        error!("Failed to connect ZeroMQ subscriber: {:?}", e);
        return;
    }

    zmq_sub.set_subscribe(b"").unwrap();

    let mut file = File::create(output_file).unwrap();

    let mut total_bytes = 0;
    let mut count = 0;
    let mut mpeg_packets = 0;
    while let Ok(msg) = zmq_sub.recv_bytes(0) {
        // Check for JSON header if enabled, it will alternate as the first message before each MPEG-TS chunk
        if send_json_header && count % 2 == 0 {
            count += 1;
            let json_header = String::from_utf8(msg.clone()).unwrap();
            if debug_on {
                info!(
                    "#{} Received JSON header: {}",
                    mpeg_packets + 1,
                    json_header
                );
            }
            continue;
        }
        total_bytes += msg.len();
        count += 1;
        mpeg_packets += 1;
        if debug_on {
            debug!(
                "#{} Received {}/{} bytes",
                mpeg_packets,
                msg.len(),
                total_bytes
            );
        } else if !silent {
            print!(".");
            std::io::stdout().flush().unwrap();
        }
        // write to file, appending if not first chunk
        file.write_all(&msg).unwrap();
    }

    // Now check the file with FFmpeg for MPEG-TS integrity
    // ... FFmpeg checking logic goes here ...
    //check_mpegts_integrity("output.ts");

    info!("Finished rscap client");
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
