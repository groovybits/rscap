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

// convert the stream data structure to the capnp format
fn capnp_to_stream_data(bytes: &[u8]) -> capnp::Result<StreamData> {
    let mut slice = bytes;
    let message_reader = capnp::serialize::read_message_from_flat_slice(
        &mut slice,
        capnp::message::ReaderOptions::new(),
    )?;

    let reader = message_reader.get_root::<stream_data_capnp::Reader>()?;

    // Convert Text to String, use an empty string in case of an error
    let stream_type = reader.get_stream_type().map_or_else(
        |_| String::new(),
        |text_reader| text_reader.to_string().unwrap_or_default(),
    );

    // Same conversion for rtp_payload_type_name
    let rtp_payload_type_name = reader.get_rtp_payload_type_name().map_or_else(
        |_| String::new(),
        |text_reader| text_reader.to_string().unwrap_or_default(),
    );

    let stream_data = StreamData {
        pid: reader.get_pid(),
        pmt_pid: reader.get_pmt_pid(),
        program_number: reader.get_program_number(),
        stream_type,
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
        rtp_payload_type_name,
        rtp_line_number: reader.get_rtp_line_number(),
        rtp_line_offset: reader.get_rtp_line_offset(),
        rtp_line_length: reader.get_rtp_line_length(),
        rtp_field_id: reader.get_rtp_field_id(),
        rtp_line_continuation: reader.get_rtp_line_continuation(),
        rtp_extended_sequence_number: reader.get_rtp_extended_sequence_number(),
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

    /// IPC Path for ZeroMQ
    #[clap(long, env = "IPC_PATH")]
    ipc_path: Option<String>,
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
    let packet_count = args.packet_count;
    let no_progress = args.no_progress;
    let output_file: String = args.output_file;
    let kafka_broker: String = args.kafka_broker;
    let kafka_topic: String = args.kafka_topic;
    let send_to_kafka = args.send_to_kafka;
    let kafka_timeout = args.kafka_timeout;
    let ipc_path = args.ipc_path;
    let mut is_ipc = false;

    // Determine the connection endpoint (IPC if provided, otherwise TCP)
    let endpoint = if let Some(ipc_path_copy) = ipc_path {
        format!("ipc://{}", ipc_path_copy)
    } else {
        format!("tcp://{}:{}", source_ip, source_port)
    };

    // check if endpoint starts with ipc
    if endpoint.starts_with("ipc://") {
        is_ipc = true;
    }

    if silent {
        // set log level to error
        std::env::set_var("RUST_LOG", "error");
    }

    // Initialize logging
    let _ = env_logger::try_init();

    // Setup ZeroMQ subscriber
    let context = async_zmq::Context::new();
    let zmq_sub = context.socket(SUB).unwrap();
    if let Err(e) = zmq_sub.connect(&endpoint) {
        error!("Failed to connect ZeroMQ subscriber: {:?}", e);
        return;
    }

    if is_ipc {
        zmq_sub.set_subscribe(b"").unwrap();
    }
    info!("ZeroMQ subscriber startup {}", endpoint);

    let mut total_bytes = 0;
    let mut mpeg_packets = 0;

    // Initialize an Option<File> to None
    let mut file = if !output_file.is_empty() {
        Some(File::create(&output_file).unwrap())
    } else {
        None
    };

    while let Ok(msg) = zmq_sub.recv_bytes(0) {
        // check for packet count
        if packet_count > 0 && mpeg_packets >= packet_count {
            break;
        }

        let more = zmq_sub.get_rcvmore().unwrap();
        let header = String::from_utf8(msg.clone()).unwrap();
        debug!(
            "Monitor: #{} Received JSON header: {}",
            mpeg_packets + 1,
            header
        );

        // Deserialize the received message into StreamData
        match capnp_to_stream_data(&msg) {
            Ok(stream_data) => {
                // Process the StreamData as needed
                // Serialize the StreamData object to JSON
                let serialized_data = serde_json::to_vec(&stream_data)
                    .expect("Failed to serialize StreamData to JSON");

                // Process the StreamData as needed
                if send_to_kafka {
                    let brokers = vec![kafka_broker.clone()];
                    let topic = kafka_topic.clone();

                    // Send serialized data to Kafka
                    match produce_message(serialized_data, topic, brokers, kafka_timeout).await {
                        Ok(_) => info!("Sent message to Kafka"),
                        Err(e) => error!("Error sending message to Kafka: {:?}", e),
                    }
                }
            }
            Err(e) => {
                error!("Error deserializing message: {:?}", e);
            }
        }

        if !no_progress {
            print!(".");
        }

        // If not expecting more parts or not receiving raw data, continue to next message
        if !more {
            continue;
        }

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
            print!("#");
        }

        // Write to file if output_file is provided
        if let Some(file) = file.as_mut() {
            file.write_all(&msg).unwrap();
        }
    }

    info!("Finished rscap monitor");
}
