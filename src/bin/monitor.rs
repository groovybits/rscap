/*
 * monitor.rs
 *
 * This is a part of a simple ZeroMQ-based MPEG-TS capture and playback system.
 * This file contains the client-side code that receives serialized metadata, or binary
 * structured packets with potentially raw MPEG-TS chunks from the rscap
 * probe and writes them to a file.
 *
 * Author: Chris Kennedy (C) 2024 LTN Global
 *
 * License: LGPL v2.1
 *
 */

use async_zmq;
use clap::Parser;
use kafka::error::Error as KafkaError;
use kafka::producer::{Producer, Record, RequiredAcks};
use log::{debug, error, info};
use std::fs::File;
use std::io::Write;
use std::time::Duration as StdDuration;
use tokio;
use tokio::time::{timeout, Duration};
use zmq::SUB;
// Include the generated paths for the Cap'n Proto schema
use capnp;
use rscap::stream_data::StreamData;
include!("../stream_data_capnp.rs");
use std::sync::Arc;

// unused code
#[allow(dead_code)]
fn capnp_to_stream_data(reader: stream_data_capnp::Reader) -> capnp::Result<StreamData> {
    let stream_data = StreamData {
        pid: reader.get_pid(),
        pmt_pid: reader.get_pmt_pid(),
        program_number: reader.get_program_number(),
        stream_type: match reader.get_stream_type() {
            Ok(text_reader) => text_reader.to_string().unwrap(),
            Err(_) => String::new(), // or handle the error as needed
        },
        continuity_counter: reader.get_continuity_counter(),
        timestamp: reader.get_timestamp(),
        bitrate: reader.get_bitrate(),
        bitrate_max: reader.get_bitrate_max(),
        bitrate_min: reader.get_bitrate_min(),
        bitrate_avg: reader.get_bitrate_avg(),
        iat: reader.get_iat(),
        iat_max: reader.get_iat_max(),
        iat_min: reader.get_iat_min(),
        iat_avg: reader.get_iat_avg(),
        error_count: reader.get_error_count(),
        last_arrival_time: reader.get_last_arrival_time(),
        start_time: reader.get_start_time(),
        total_bits: reader.get_total_bits(),
        count: reader.get_count(),
        rtp_timestamp: reader.get_rtp_timestamp(),
        rtp_payload_type: reader.get_rtp_payload_type(),
        rtp_payload_type_name: match reader.get_rtp_payload_type_name() {
            Ok(text_reader) => text_reader.to_string().unwrap(),
            Err(_) => String::new(), // or handle the error as needed
        },
        rtp_line_number: reader.get_rtp_line_number(),
        rtp_line_offset: reader.get_rtp_line_offset(),
        rtp_line_length: reader.get_rtp_line_length(),
        rtp_field_id: reader.get_rtp_field_id(),
        rtp_line_continuation: reader.get_rtp_line_continuation(),
        rtp_extended_sequence_number: reader.get_rtp_extended_sequence_number().into(),
        packet: Arc::new(Vec::new()),
        packet_start: 0,
        packet_len: 0,
    };

    Ok(stream_data)
}

async fn produce_message(
    data: Vec<u8>, // Changed to Vec<u8> to allow cloning
    topic: String,
    brokers: Vec<String>,
    kafka_timeout: u64,
) -> Result<(), KafkaError> {
    let kafka_operation_timeout = Duration::from_secs(kafka_timeout);

    match timeout(
        kafka_operation_timeout,
        tokio::task::spawn_blocking(move || {
            let mut producer = Producer::from_hosts(brokers)
                .with_ack_timeout(StdDuration::from_secs(1))
                .with_required_acks(RequiredAcks::One)
                .create()?;

            producer.send(&Record {
                topic: &topic,
                partition: -1,
                key: (),
                value: data, // Pass Vec<u8> directly
            })?;

            Ok::<(), KafkaError>(())
        }),
    )
    .await
    {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(e))) => Err(e),
        Ok(Err(e)) => Err(KafkaError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("JoinError: {}", e),
        ))),
        Err(_) => Err(KafkaError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "Kafka operation timed out",
        ))),
    }
}

#[derive(Parser, Debug)]
#[clap(
    author = "Chris Kennedy",
    version = "1.1",
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

    /// Sets if header should be recieved
    #[clap(long, env = "RECV_HEADER", default_value_t = false)]
    recv_header: bool,

    /// Sets if Raw Stream should be sent
    #[clap(long, env = "RECV_RAW_STREAM", default_value_t = false)]
    recv_raw_stream: bool,

    /// number of packets to capture
    #[clap(long, env = "PACKET_COUNT", default_value_t = 0)]
    packet_count: u64,

    /// Turn off progress output dots
    #[clap(long, env = "NO_PROGRESS", default_value_t = false)]
    no_progress: bool,

    /// Output Filename
    #[clap(long, env = "OUTPUT_FILE", default_value = "")]
    output_file: String,

    /// Kafka Broker
    #[clap(long, env = "KAFKA_BROKER", default_value = "localhost:9092")]
    kafka_broker: String,

    /// Kafka Topic
    #[clap(long, env = "KAFKA_TOPIC", default_value = "rscap")]
    kafka_topic: String,

    /// Send to Kafka if true
    #[clap(long, env = "SEND_TO_KAFKA", default_value_t = false)]
    send_to_kafka: bool,

    /// Kafka timeout to drop packets
    #[clap(long, env = "KAFKA_TIMEOUT", default_value_t = 0)]
    kafka_timeout: u64,
}

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok(); // read .env file

    println!("RsCap Monitor for ZeroMQ input of MPEG-TS and SMPTE 2110 streams from remote probe.");

    let args = Args::parse();

    // Use the parsed arguments directly
    let source_port = args.source_port;
    let source_ip = args.source_ip;
    /*let debug_on = args.debug_on;*/
    // TODO: implement frame hex dumps, move from probe and test capture with them.
    let silent = args.silent;
    let recv_header = args.recv_header;
    let recv_raw_stream = args.recv_raw_stream;
    let packet_count = args.packet_count;
    let no_progress = args.no_progress;
    let output_file: String = args.output_file;
    let kafka_broker: String = args.kafka_broker;
    let kafka_topic: String = args.kafka_topic;
    let send_to_kafka = args.send_to_kafka;

    if silent {
        // set log level to error
        std::env::set_var("RUST_LOG", "error");
    }

    // Initialize logging
    let _ = env_logger::try_init();

    // Setup ZeroMQ subscriber
    let context = async_zmq::Context::new();
    let zmq_sub = context.socket(SUB).unwrap();
    let source_port_ip = format!("tcp://{}:{}", source_ip, source_port);
    if let Err(e) = zmq_sub.connect(&source_port_ip) {
        error!("Failed to connect ZeroMQ subscriber: {:?}", e);
        return;
    }

    zmq_sub.set_subscribe(b"").unwrap();

    let mut total_bytes = 0;
    let mut mpeg_packets = 0;
    let mut expecting_metadata = recv_header; // Expect metadata only if recv_header is true
                                              // Initialize an Option<File> to None
    let mut file = if !output_file.is_empty() {
        Some(File::create(&output_file).unwrap())
    } else {
        None
    };

    while let Ok(msg) = zmq_sub.recv_bytes(0) {
        let more = zmq_sub.get_rcvmore().unwrap();

        if expecting_metadata {
            // Process serialized header if expecting metadata
            if recv_header {
                let header = String::from_utf8(msg.clone()).unwrap();
                debug!(
                    "Monitor: #{} Received JSON header: {}",
                    mpeg_packets + 1,
                    header
                );

                if send_to_kafka {
                    let brokers = vec![kafka_broker.clone()];
                    let topic = kafka_topic.clone();
                    let data = msg.clone(); // Clone the data
                    let kafka_timeout = args.kafka_timeout;

                    match produce_message(data, topic, brokers, kafka_timeout).await {
                        Ok(_) => info!("Sent message to Kafka"),
                        Err(e) => error!("Error sending message to Kafka: {:?}", e),
                    }
                }

                if !no_progress {
                    print!("*");
                    //std::io::stdout().flush().unwrap();
                }
            }

            // If not expecting more parts or not receiving raw data, continue to next message
            if !more || !recv_raw_stream {
                expecting_metadata = recv_header; // Reset for next message if applicable
                continue;
            }

            expecting_metadata = false; // Next message will be raw data
        } else {
            // Process raw data packet
            total_bytes += msg.len();
            mpeg_packets += 1;

            debug!(
                "Monitor: #{} Received {}/{} bytes",
                mpeg_packets,
                msg.len(),
                total_bytes
            );

            if !no_progress {
                print!(".");
                //std::io::stdout().flush().unwrap();
            }

            // check for packet count
            if packet_count > 0 && mpeg_packets >= packet_count {
                break;
            }

            // Write to file if output_file is provided
            if let Some(file) = file.as_mut() {
                file.write_all(&msg).unwrap();
            }

            expecting_metadata = recv_header; // Reset for next message if applicable
        }
    }

    info!("Finished rscap monitor");
}
