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
use clap::Parser;
use log::{debug, error, info};
use std::env;
use std::fs::File;
use std::io::Write;
use tokio;


#[derive(Parser, Debug)]
#[clap(
    author = "Chris Kennedy",
    version = "1.0",
    about = "RsCap Monitor for ZeroMQ input of MPEG-TS and SMPTE 2110 streams from remote probe."
)]
struct Args {
    /// Sets the target port
    #[clap(long, env = "TARGET_PORT", default_value_t = 5556)]
    source_port: i32,

    /// Sets the target IP
    #[clap(long, env = "TARGET_IP", default_value = "127.0.0.1")]
    source_ip: String,

    /// Sets the debug mode
    #[clap(long, env = "DEBUG", default_value_t = false)]
    debug_on: bool,

    /// Sets the silent mode
    #[clap(long, env = "SILENT", default_value_t = false)]
    silent: bool,

    /// Sets if JSON header should be sent
    #[clap(long, env = "RECV_JSON_HEADER", default_value_t = false)]
    recv_json_header: bool,

    /// Sets if Raw Stream should be sent
    #[clap(long, env = "RECV_RAW_STREAM", default_value_t = false)]
    recv_raw_stream: bool,

    /// number of packets to capture
    #[clap(long, env = "PACKET_COUNT", default_value_t = 0)]
    packet_count: u64,

    /// Turn off progress output dots
    #[clap(long, env = "NO_PROGRESS", default_value_t = false)]
    no_progress: bool,

    /// Force smpte2110 mode
    #[clap(long, env = "SMPT2110", default_value_t = false)]
    smpte2110: bool,

    /// Output Filename
    #[clap(long, env = "OUTPUT_FILE", default_value = "output.ts")]
    output_file: String,
}

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok(); // read .env file

    println!("RsCap Monitor for ZeroMQ input of MPEG-TS and SMPTE 2110 streams from remote probe.");

    let args = Args::parse();

    // Use the parsed arguments directly
    let source_port = args.source_port;
    let source_ip = args.source_ip;
    let debug_on = args.debug_on;
    let silent = args.silent;
    let recv_json_header = args.recv_json_header;
    let recv_raw_stream = args.recv_raw_stream;
    let packet_count = args.packet_count;
    let no_progress = args.no_progress;
    let smpte2110 = args.smpte2110;
    let output_file: String = args.output_file;

    if silent {
        // set log level to error
        std::env::set_var("RUST_LOG", "error");
    }

    // Initialize logging
    let _ = env_logger::try_init();

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
    let mut mpeg_packets = 0;
    let mut expecting_metadata = recv_json_header; // Expect metadata only if recv_json_header is true

    while let Ok(msg) = zmq_sub.recv_bytes(0) {
        if expecting_metadata {
            let json_header = String::from_utf8(msg.clone()).unwrap();
            if debug_on {
                info!(
                    "#{} Received JSON header: {}",
                    mpeg_packets + 1,
                    json_header
                );
            }
            expecting_metadata = false; // Next message will be data
        } else {
            total_bytes += msg.len();
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

            // check for packet count
            if packet_count > 0 && mpeg_packets >= packet_count {
                break;
            }

            // write to file, appending if not first chunk
            file.write_all(&msg).unwrap();

            if recv_json_header {
                expecting_metadata = true; // Expect metadata again if recv_json_header is true
            }
        }
    }

    info!("Finished rscap client");
}