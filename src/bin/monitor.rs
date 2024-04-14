/*
 * monitor.rs
 *
 * This is a part of a simple ZeroMQ-based MPEG-TS capture and playback system.
 * This file contains the client-side code that receives serialized metadata, or binary
 * structured packets with potentially raw MPEG-TS chunks from the rscap
 * probe and writes them to a file.
 *
 * Author: Chris Kennedy (C) 2024
 *
 * License: MIT
 *
 */

use async_zmq;
use base64::{engine::general_purpose, Engine as _};
use clap::Parser;
use env_logger::{Builder, Env};
use log::{debug, error, info, warn};
use rdkafka::admin::{AdminClient, AdminOptions, NewTopic};
use rdkafka::client::DefaultClientContext;
use rdkafka::config::ClientConfig;
use rdkafka::producer::{FutureProducer, FutureRecord};
use std::fs::File;
use std::io::Write;
use std::time::Instant;
use tokio;
use tokio::time::Duration;
use zmq::PULL;
// Include the generated paths for the Cap'n Proto schema
use capnp;
use rscap::hexdump;
use rscap::stream_data::StreamData;
include!("../stream_data_capnp.rs");
use rscap::{get_stats_as_json, StatsType};
use std::sync::Arc;
// Video Processor Decoder
use ahash::AHashMap;
use lazy_static::lazy_static;
use rscap::current_unix_timestamp_ms;
use serde_json::{json, Value};
use std::sync::RwLock;

lazy_static! {
    static ref STREAM_GROUPINGS: RwLock<AHashMap<u16, StreamGrouping>> =
        RwLock::new(AHashMap::new());
}

struct StreamGrouping {
    stream_data_list: Vec<StreamData>,
}

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

    let source_ip = reader.get_source_ip().map_or_else(
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
        current_error_count: reader.get_current_error_count(),
        last_arrival_time: reader.get_last_arrival_time(),
        last_sample_time: reader.get_last_sample_time(),
        start_time: reader.get_start_time(),
        total_bits: reader.get_total_bits(),
        total_bits_sample: reader.get_total_bits_sample(),
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
        stream_type_number: reader.get_stream_type_number(),
        capture_time: reader.get_capture_time(),
        capture_iat: reader.get_capture_iat(),
        source_ip,
        source_port: reader.get_source_port() as i32,

        // System stats fields
        total_memory: reader.get_total_memory(),
        used_memory: reader.get_used_memory(),
        total_swap: reader.get_total_swap(),
        used_swap: reader.get_used_swap(),
        cpu_usage: reader.get_cpu_usage(),
        cpu_count: reader.get_cpu_count() as usize,
        core_count: reader.get_core_count() as usize,
        boot_time: reader.get_boot_time(),
        load_avg_one: reader.get_load_avg_one(),
        load_avg_five: reader.get_load_avg_five(),
        load_avg_fifteen: reader.get_load_avg_fifteen(),
        host_name: reader.get_host_name()?.to_string()?,
        kernel_version: reader.get_kernel_version()?.to_string()?,
        os_version: reader.get_os_version()?.to_string()?,
        has_image: reader.get_has_image(),
        image_pts: reader.get_image_pts(),
        capture_iat_max: reader.get_capture_iat_max(),
        log_message: reader.get_log_message()?.to_string()?,
        probe_id: reader.get_probe_id()?.to_string()?,
        captions: reader.get_captions()?.to_string()?,
        pid_map: reader.get_pid_map()?.to_string()?,
        scte35: reader.get_scte35()?.to_string()?,
        audio_loudness: reader.get_audio_loudness()?.to_string()?,
    };

    Ok(stream_data)
}

// Callback function to check the status of the sent messages
async fn delivery_report(
    result: Result<(i32, i64), (rdkafka::error::KafkaError, rdkafka::message::OwnedMessage)>,
) {
    match result {
        Ok((partition, offset)) => debug!(
            "Message delivered to partition {} at offset {}",
            partition, offset
        ),
        Err((err, _)) => println!("Message delivery failed: {:?}", err),
    }
}

fn flatten_streams(
    stream_groupings: &AHashMap<u16, StreamGrouping>,
) -> serde_json::Map<String, Value> {
    let mut flat_structure: serde_json::Map<String, Value> = serde_json::Map::new();

    for (pid, grouping) in stream_groupings.iter() {
        let stream_data = grouping.stream_data_list.last().unwrap(); // Assuming last item is representative

        let prefix = format!("streams.{}", pid);

        // Adding basic info of the stream
        flat_structure.insert(
            format!("{}.stream_type", prefix),
            json!(stream_data.stream_type),
        );
        flat_structure.insert(
            format!("{}.program_number", prefix),
            json!(stream_data.program_number),
        );
        flat_structure.insert(format!("{}.pmt_pid", prefix), json!(stream_data.pmt_pid));
        flat_structure.insert(
            format!("{}.bitrate_avg", prefix),
            json!(stream_data.bitrate_avg),
        );
        flat_structure.insert(format!("{}.iat_avg", prefix), json!(stream_data.iat_avg));
        flat_structure.insert(
            format!("{}.packet_count", prefix),
            json!(grouping.stream_data_list.len()),
        );

        // Adding detailed stats of the stream
        flat_structure.insert(format!("{}.pid", prefix), json!(stream_data.pid));
        flat_structure.insert(format!("{}.pmt_pid", prefix), json!(stream_data.pmt_pid));
        flat_structure.insert(
            format!("{}.program_number", prefix),
            json!(stream_data.program_number),
        );
        flat_structure.insert(
            format!("{}.stream_type", prefix),
            json!(stream_data.stream_type),
        );
        flat_structure.insert(
            format!("{}.continuity_counter", prefix),
            json!(stream_data.continuity_counter),
        );
        flat_structure.insert(
            format!("{}.timestamp", prefix),
            json!(stream_data.timestamp),
        );
        flat_structure.insert(format!("{}.bitrate", prefix), json!(stream_data.bitrate));
        flat_structure.insert(
            format!("{}.bitrate_max", prefix),
            json!(stream_data.bitrate_max),
        );
        flat_structure.insert(
            format!("{}.bitrate_min", prefix),
            json!(stream_data.bitrate_min),
        );
        flat_structure.insert(
            format!("{}.bitrate_avg", prefix),
            json!(stream_data.bitrate_avg),
        );
        flat_structure.insert(format!("{}.iat", prefix), json!(stream_data.iat));
        flat_structure.insert(format!("{}.iat_max", prefix), json!(stream_data.iat_max));
        flat_structure.insert(format!("{}.iat_min", prefix), json!(stream_data.iat_min));
        flat_structure.insert(format!("{}.iat_avg", prefix), json!(stream_data.iat_avg));
        flat_structure.insert(
            format!("{}.error_count", prefix),
            json!(stream_data.error_count),
        );
        flat_structure.insert(
            format!("{}.current_error_count", prefix),
            json!(stream_data.current_error_count),
        );
        flat_structure.insert(
            format!("{}.last_arrival_time", prefix),
            json!(stream_data.last_arrival_time),
        );
        flat_structure.insert(
            format!("{}.last_sample_time", prefix),
            json!(stream_data.last_sample_time),
        );
        flat_structure.insert(
            format!("{}.start_time", prefix),
            json!(stream_data.start_time),
        );
        flat_structure.insert(
            format!("{}.total_bits", prefix),
            json!(stream_data.total_bits),
        );
        flat_structure.insert(
            format!("{}.total_bits_sample", prefix),
            json!(stream_data.total_bits_sample),
        );
        flat_structure.insert(format!("{}.count", prefix), json!(stream_data.count));
        flat_structure.insert(
            format!("{}.packet_start", prefix),
            json!(stream_data.packet_start),
        );
        flat_structure.insert(
            format!("{}.packet_len", prefix),
            json!(stream_data.packet_len),
        );
        flat_structure.insert(
            format!("{}.rtp_timestamp", prefix),
            json!(stream_data.rtp_timestamp),
        );
        flat_structure.insert(
            format!("{}.rtp_payload_type", prefix),
            json!(stream_data.rtp_payload_type),
        );
        flat_structure.insert(
            format!("{}.rtp_payload_type_name", prefix),
            json!(stream_data.rtp_payload_type_name),
        );
        flat_structure.insert(
            format!("{}.rtp_line_number", prefix),
            json!(stream_data.rtp_line_number),
        );
        flat_structure.insert(
            format!("{}.rtp_line_offset", prefix),
            json!(stream_data.rtp_line_offset),
        );
        flat_structure.insert(
            format!("{}.rtp_line_length", prefix),
            json!(stream_data.rtp_line_length),
        );
        flat_structure.insert(
            format!("{}.rtp_field_id", prefix),
            json!(stream_data.rtp_field_id),
        );
        flat_structure.insert(
            format!("{}.rtp_line_continuation", prefix),
            json!(stream_data.rtp_line_continuation),
        );
        flat_structure.insert(
            format!("{}.rtp_extended_sequence_number", prefix),
            json!(stream_data.rtp_extended_sequence_number),
        );
        flat_structure.insert(
            format!("{}.stream_type_number", prefix),
            json!(stream_data.stream_type_number),
        );
        flat_structure.insert(
            format!("{}.capture_time", prefix),
            json!(stream_data.capture_time),
        );
        flat_structure.insert(
            format!("{}.capture_iat", prefix),
            json!(stream_data.capture_iat),
        );
        flat_structure.insert(
            format!("{}.capture_iat_max", prefix),
            json!(stream_data.capture_iat_max),
        );

        // Add system stats fields to the flattened structure
        flat_structure.insert(
            format!("{}.total_memory", prefix),
            json!(stream_data.total_memory),
        );
        flat_structure.insert(
            format!("{}.used_memory", prefix),
            json!(stream_data.used_memory),
        );
        flat_structure.insert(
            format!("{}.total_swap", prefix),
            json!(stream_data.total_swap),
        );
        flat_structure.insert(
            format!("{}.used_swap", prefix),
            json!(stream_data.used_swap),
        );
        flat_structure.insert(
            format!("{}.cpu_usage", prefix),
            json!(stream_data.cpu_usage),
        );
        flat_structure.insert(
            format!("{}.cpu_count", prefix),
            json!(stream_data.cpu_count),
        );
        flat_structure.insert(
            format!("{}.core_count", prefix),
            json!(stream_data.core_count),
        );
        flat_structure.insert(
            format!("{}.boot_time", prefix),
            json!(stream_data.boot_time),
        );
        flat_structure.insert(
            format!("{}.load_avg_one", prefix),
            json!(stream_data.load_avg_one),
        );
        flat_structure.insert(
            format!("{}.load_avg_five", prefix),
            json!(stream_data.load_avg_five),
        );
        flat_structure.insert(
            format!("{}.load_avg_fifteen", prefix),
            json!(stream_data.load_avg_fifteen),
        );
        flat_structure.insert(
            format!("{}.host_name", prefix),
            json!(stream_data.host_name),
        );
        flat_structure.insert(
            format!("{}.kernel_version", prefix),
            json!(stream_data.kernel_version),
        );
        flat_structure.insert(
            format!("{}.os_version", prefix),
            json!(stream_data.os_version),
        );
        flat_structure.insert(
            format!("{}.has_image", prefix),
            json!(stream_data.has_image),
        );
        flat_structure.insert(
            format!("{}.image_pts", prefix),
            json!(stream_data.image_pts),
        );
        flat_structure.insert(
            format!("{}.log_message", prefix),
            json!(stream_data.log_message),
        );
        flat_structure.insert(format!("{}.id", prefix), json!(stream_data.probe_id));
        flat_structure.insert(format!("{}.captions", prefix), json!(stream_data.captions));
    }

    flat_structure
}

async fn produce_message(
    data: Vec<u8>,
    kafka_server: String,
    kafka_topic: String,
    kafka_timeout: u64,
    key: String,
    _stream_data_timestamp: i64,
    producer: FutureProducer,
    admin_client: &AdminClient<DefaultClientContext>,
) {
    debug!("Service {} sending message", kafka_topic);
    let kafka_topic = kafka_topic.replace(":", "_").replace(".", "_");

    // Metadata fetching is problematic, directly attempt to ensure the topic exists.
    // This code block tries to create the topic if it doesn't already exist, ignoring errors that indicate existence.
    let new_topic = NewTopic::new(&kafka_topic, 1, rdkafka::admin::TopicReplication::Fixed(1));
    let _ = admin_client
        .create_topics(&[new_topic], &AdminOptions::new())
        .await;

    debug!(
        "Forwarding message for topic {} to Kafka server {:?}",
        kafka_topic, kafka_server
    );

    let record = FutureRecord::to(&kafka_topic).payload(&data).key(&key);
    /* .timestamp(stream_data_timestamp);*/

    let delivery_future = producer
        .send(record, Duration::from_secs(kafka_timeout))
        .await;
    match delivery_future {
        Ok(delivery_result) => delivery_report(Ok(delivery_result)).await,
        Err(e) => error!("Failed to send message: {:?}", e),
    }
}

#[derive(Parser, Debug)]
#[clap(
    author = "Chris Kennedy",
    version = "0.5.38",
    about = "RsCap Monitor for ZeroMQ input of MPEG-TS and SMPTE 2110 streams from remote probe."
)]
struct Args {
    /// Sets the target port
    #[clap(long, env = "SOURCE_PORT", default_value_t = 5556)]
    source_port: i32,

    /// Sets the target IP
    #[clap(long, env = "SOURCE_IP", default_value = "127.0.0.1")]
    source_ip: String,

    /// Sets the packet size
    #[clap(long, env = "PACKET_SIZE", default_value_t = 188)]
    packet_size: usize,

    /// Sets the debug mode
    #[clap(long, env = "DEBUG", default_value_t = false)]
    debug_on: bool,

    /// Sets the silent mode
    #[clap(long, env = "SILENT", default_value_t = false)]
    silent: bool,

    /// Sets if Raw Stream will be received
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
    #[clap(long, env = "KAFKA_TIMEOUT", default_value_t = 100)]
    kafka_timeout: u64,

    /// Kafka Key
    #[clap(long, env = "KAFKA_KEY", default_value = "")]
    kafka_key: String,

    /// IPC Path for ZeroMQ
    #[clap(long, env = "IPC_PATH")]
    ipc_path: Option<String>,

    /// Show OS
    #[clap(long, env = "SHOW_OS_STATS", default_value_t = false)]
    show_os_stats: bool,

    // Add the new argument for Kafka interval
    /// Kafka sending interval in milliseconds
    #[clap(long, env = "KAFKA_INTERVAL", default_value_t = 1000)]
    kafka_interval: u64,

    /// Loglevel
    #[clap(long, env = "LOGLEVEL", default_value = "info")]
    loglevel: String,
}

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok(); // read .env file

    let args = Args::parse();

    // Use the parsed arguments directly
    let source_port = args.source_port;
    let source_ip = args.source_ip;
    let debug_on = args.debug_on;
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
    let show_os_stats = args.show_os_stats;
    let kafka_key = args.kafka_key;

    println!("RsCap Monitor starting up...");

    // Determine the connection endpoint (IPC if provided, otherwise TCP)
    let endpoint = if let Some(ipc_path_copy) = ipc_path {
        format!("ipc://{}", ipc_path_copy)
    } else {
        format!("tcp://{}:{}", source_ip, source_port)
    };

    if silent {
        // set log level to error
        std::env::set_var("RUST_LOG", "error");
    }

    // Set Rust log level with --loglevel if it is set
    let loglevel = args.loglevel.to_lowercase();
    match loglevel.as_str() {
        "error" => {
            log::set_max_level(log::LevelFilter::Error);
        }
        "warn" => {
            log::set_max_level(log::LevelFilter::Warn);
        }
        "info" => {
            log::set_max_level(log::LevelFilter::Info);
        }
        "debug" => {
            log::set_max_level(log::LevelFilter::Debug);
        }
        "trace" => {
            log::set_max_level(log::LevelFilter::Trace);
        }
        _ => {
            log::set_max_level(log::LevelFilter::Info);
        }
    }

    // Initialize logging
    let env = Env::default().filter_or("RUST_LOG", loglevel.as_str()); // Default to `info` if `RUST_LOG` is not set
    Builder::from_env(env).init();

    // Setup ZeroMQ subscriber
    let context = async_zmq::Context::new();
    let zmq_sub = context.socket(PULL).unwrap();
    if let Err(e) = zmq_sub.connect(&endpoint) {
        error!("Failed to connect ZeroMQ subscriber: {:?}", e);
        return;
    }
    info!("ZeroMQ subscriber startup {}", endpoint);

    let mut total_bytes = 0;
    let mut counter = 0;

    let mut dot_last_file_write = Instant::now();
    let mut dot_last_sent_stats = Instant::now();
    let mut dot_last_sent_ts = Instant::now();
    let mut last_kafka_send_time = Instant::now();

    let mut kafka_conf = ClientConfig::new();
    kafka_conf.set("bootstrap.servers", &kafka_broker);
    kafka_conf.set("client.id", "rscap");

    let admin_client: AdminClient<DefaultClientContext> =
        kafka_conf.create().expect("Failed to create admin client");
    let producer: FutureProducer = kafka_conf
        .create()
        .expect("Failed to create Kafka producer");

    // OS and Network stats
    let mut system_stats_json = if show_os_stats {
        get_stats_as_json(StatsType::System).await
    } else {
        json!({})
    };

    if system_stats_json != json!({}) {
        info!("Startup System OS Stats:\n{:?}", system_stats_json);
    }

    let mut output_file_counter = 0;
    let mut log_messages = Vec::new();

    loop {
        // check for packet count
        if packet_count > 0 && counter >= packet_count {
            break;
        }

        if !no_progress && dot_last_sent_ts.elapsed().as_secs() > 1 {
            dot_last_sent_ts = Instant::now();
            print!(".");
            // flush stdout
            std::io::stdout().flush().unwrap();
        }

        // Attempt to receive a message, but do not block if unavailable
        match zmq_sub.recv_multipart(0) {
            Ok(packet_msg) if !packet_msg.is_empty() => {
                // get first message
                let header_msg = packet_msg[0].clone();

                if show_os_stats && dot_last_sent_stats.elapsed().as_secs() > 10 {
                    dot_last_sent_stats = Instant::now();

                    // OS and Network stats
                    system_stats_json = get_stats_as_json(StatsType::System).await;

                    if show_os_stats && system_stats_json != json!({}) {
                        info!("System stats as JSON:\n{:?}", system_stats_json);
                    }
                }

                // Deserialize the received message into StreamData
                match capnp_to_stream_data(&header_msg) {
                    Ok(stream_data) => {
                        // print the structure of the packet
                        debug!("MONITOR::PACKET:RECEIVE[{}] pid: {} stream_type: {} bitrate: {} bitrate_max: {} bitrate_min: {} bitrate_avg: {} iat: {} iat_max: {} iat_min: {} iat_avg: {} errors: {} continuity_counter: {} timestamp: {}",
                                    counter + 1,
                                    stream_data.pid,
                                    stream_data.stream_type,
                                    stream_data.bitrate,
                                    stream_data.bitrate_max,
                                    stream_data.bitrate_min,
                                    stream_data.bitrate_avg,
                                    stream_data.iat,
                                    stream_data.iat_max,
                                    stream_data.iat_min,
                                    stream_data.iat_avg,
                                    stream_data.error_count,
                                    stream_data.continuity_counter,
                                    stream_data.timestamp,
                                );

                        // get data message
                        let data_msg = packet_msg[1].clone();

                        // Process raw data packet
                        total_bytes += data_msg.len();
                        debug!(
                            "Monitor: #{} Received {}/{} bytes",
                            counter,
                            data_msg.len(),
                            total_bytes
                        );

                        if debug_on {
                            let data_msg_arc = Arc::new(data_msg.to_vec());
                            hexdump(&data_msg_arc, 0, data_msg.len());
                        }

                        let mut base64_image = String::new();

                        // Check if Decoding or if Demuxing
                        if args.recv_raw_stream {
                            // Initialize an Option<File> to None
                            let mut output_file_mut = if !output_file.is_empty() {
                                Some(File::create(&output_file).unwrap())
                            } else {
                                None
                            };

                            // Write to file if output_file is provided
                            if let Some(file) = output_file_mut.as_mut() {
                                output_file_counter += 1;
                                if !no_progress && dot_last_file_write.elapsed().as_secs() > 1 {
                                    dot_last_file_write = Instant::now();
                                    print!("*");
                                    // flush stdout
                                    std::io::stdout().flush().unwrap();
                                }
                                file.write_all(&data_msg).unwrap();
                            }
                        } else {
                            // change output_file_name_mut to contain an incrementing _00000000.jpg ending
                            // use output_file_counter to increment the file name
                            // example output_file_name_mut = "output_{:08}.jpg", output_file_counter
                            // remove existing .jpg if given first
                            let output_file_without_jpg = output_file.replace(".jpg", "");
                            if data_msg.len() > 0 && stream_data.has_image > 0 {
                                let output_file_incremental = format!(
                                    "{}_{:08}.jpg",
                                    output_file_without_jpg, output_file_counter
                                );

                                let mut output_file_mut = if !output_file.is_empty() {
                                    Some(File::create(&output_file_incremental).unwrap())
                                } else {
                                    None
                                };

                                info!(
                                    "Monitor: Jpeg image received: {} size {} pts saved to {}",
                                    data_msg.len(),
                                    stream_data.image_pts,
                                    output_file_incremental
                                );

                                // Write to file if output_file is provided
                                if let Some(file) = output_file_mut.as_mut() {
                                    output_file_counter += 1;
                                    if !no_progress && dot_last_file_write.elapsed().as_secs() > 1 {
                                        dot_last_file_write = Instant::now();
                                        print!("*");
                                        // flush stdout
                                        std::io::stdout().flush().unwrap();
                                    }
                                    file.write_all(&data_msg).unwrap();
                                }

                                // Encode the JPEG image as Base64
                                base64_image = general_purpose::STANDARD.encode(&data_msg);
                            }
                        }

                        let pid = stream_data.pid;
                        {
                            let mut stream_groupings = STREAM_GROUPINGS.write().unwrap();
                            if let Some(grouping) = stream_groupings.get_mut(&pid) {
                                // Update the existing StreamData instance in the grouping
                                let last_stream_data =
                                    grouping.stream_data_list.last_mut().unwrap();
                                *last_stream_data = stream_data.clone();
                            } else {
                                let new_grouping = StreamGrouping {
                                    stream_data_list: vec![stream_data.clone()],
                                };
                                stream_groupings.insert(pid, new_grouping);
                            }
                        }

                        // Check if it's time to send data to Kafka based on the interval
                        if send_to_kafka {
                            // Acquire read access to STREAM_GROUPINGS
                            let stream_groupings = STREAM_GROUPINGS.read().unwrap();
                            let mut flattened_data = flatten_streams(&stream_groupings);

                            // Initialize variables to accumulate global averages
                            let mut total_bitrate_avg: u64 = 0;
                            let mut total_iat_avg: u64 = 0;
                            let mut total_iat_max: u64 = 0;
                            let mut total_cc_errors: u64 = 0;
                            let mut total_cc_errors_current: u64 = 0;
                            let mut stream_count: u64 = 0;
                            let mut source_ip: String = String::new();
                            let mut source_port: u32 = 0;
                            let mut image_pts: u64 = 0;
                            let mut probe_id: String = String::new();
                            let mut captions: String = String::new();
                            let mut pid_map: String = String::new();
                            let mut scte35: String = String::new();
                            let mut audio_loudness: String = String::new();

                            // Process each stream to accumulate averages
                            for (_, grouping) in stream_groupings.iter() {
                                for stream_data in &grouping.stream_data_list {
                                    total_bitrate_avg += stream_data.bitrate_avg as u64;
                                    total_iat_avg += stream_data.capture_iat;
                                    total_iat_max += stream_data.capture_iat_max;
                                    total_cc_errors += stream_data.error_count as u64;
                                    total_cc_errors_current +=
                                        stream_data.current_error_count as u64;
                                    source_port = stream_data.source_port as u32;
                                    source_ip = stream_data.source_ip.clone();
                                    if stream_data.has_image > 0 && stream_data.image_pts > 0 {
                                        image_pts = stream_data.image_pts;
                                    }
                                    if stream_data.log_message != "" {
                                        info!("Got Log Message: {}", stream_data.log_message);
                                        log_messages.push(stream_data.log_message.clone());
                                    }
                                    if stream_data.probe_id != "" {
                                        if probe_id != "" && probe_id != stream_data.probe_id {
                                            warn!(
                                                "Multiple probe IDs detected: {} and {}",
                                                probe_id, stream_data.probe_id
                                            );
                                        }
                                        probe_id = stream_data.probe_id.clone();
                                    }
                                    if stream_data.captions != "" {
                                        // concatenate captions
                                        captions = format!("{}{}", captions, stream_data.captions);
                                    }
                                    if stream_data.pid_map != "" {
                                        // concatenate pid_map
                                        pid_map = format!("{}{}", pid_map, stream_data.pid_map);
                                    }
                                    if stream_data.scte35 != "" {
                                        // concatenate scte35
                                        scte35 = format!("{}{}", scte35, stream_data.scte35);
                                    }
                                    if stream_data.audio_loudness != "" {
                                        // concatenate audio_loudness
                                        audio_loudness = format!(
                                            "{}{}",
                                            audio_loudness, stream_data.audio_loudness
                                        );
                                    }
                                    stream_count += 1;
                                }
                            }

                            // Continuity Counter errors
                            let global_cc_errors = total_cc_errors;
                            let global_cc_errors_current = total_cc_errors_current;

                            // avg IAT
                            let global_iat_avg = if stream_count > 0 {
                                total_iat_avg as f64 / stream_count as f64
                            } else {
                                0.0
                            };

                            // max IAT
                            let global_iat_max = if stream_count > 0 {
                                total_iat_max as f64 / stream_count as f64
                            } else {
                                0.0
                            };

                            // Calculate global averages
                            let global_bitrate_avg = if stream_count > 0 {
                                total_bitrate_avg
                            } else {
                                0
                            };
                            let current_timestamp = current_unix_timestamp_ms().unwrap_or(0); // stream_data.capture_time;

                            // Directly insert global statistics and timestamp into the flattened_data map
                            flattened_data.insert(
                                "bitrate_avg_global".to_string(),
                                serde_json::json!(global_bitrate_avg),
                            );
                            flattened_data.insert(
                                "iat_avg_global".to_string(),
                                serde_json::json!(global_iat_avg),
                            );
                            flattened_data.insert(
                                "iat_max_global".to_string(),
                                serde_json::json!(global_iat_max),
                            );
                            flattened_data.insert(
                                "cc_errors_global".to_string(),
                                serde_json::json!(global_cc_errors),
                            );
                            flattened_data.insert(
                                "current_cc_errors_global".to_string(),
                                serde_json::json!(global_cc_errors_current),
                            );
                            flattened_data.insert(
                                "timestamp".to_string(),
                                serde_json::json!(current_timestamp),
                            );
                            flattened_data
                                .insert("source_ip".to_string(), serde_json::json!(source_ip));
                            flattened_data
                                .insert("source_port".to_string(), serde_json::json!(source_port));
                            flattened_data
                                .insert("captions".to_string(), serde_json::json!(captions));

                            let mut force_send_message = false;

                            if captions != "" {
                                force_send_message = true;
                            }
                            flattened_data
                                .insert("captions".to_string(), serde_json::json!(captions));

                            // Insert the base64_image field into the flattened_data map
                            flattened_data
                                .insert("image_pts".to_string(), serde_json::json!(image_pts));
                            let base64_image_tag = if base64_image != "" {
                                info!("Got Image: {} bytes", base64_image.len());
                                force_send_message = true;
                                format!("data:image/jpeg;base64,{}", base64_image)
                            } else {
                                "".to_string()
                            };
                            flattened_data.insert(
                                "base64_image".to_string(),
                                serde_json::json!(base64_image_tag),
                            );
                            // Check if we have a log_message in log_messages Vector, if so add it to the flattened_data map
                            if !log_messages.is_empty() {
                                force_send_message = true;
                                // remove one log message from the log_messages array
                                let log_message = log_messages.pop().unwrap();
                                flattened_data.insert(
                                    "log_message".to_string(),
                                    serde_json::json!(log_message),
                                );
                            } else {
                                flattened_data
                                    .insert("log_message".to_string(), serde_json::json!(""));
                            }
                            // probe id
                            flattened_data.insert("id".to_string(), serde_json::json!(probe_id));

                            // insert the pid_map, scte35, and audio_loudness fields into the flattened_data map
                            flattened_data
                                .insert("pid_map".to_string(), serde_json::json!(pid_map));
                            flattened_data.insert("scte35".to_string(), serde_json::json!(scte35));
                            flattened_data.insert(
                                "audio_loudness".to_string(),
                                serde_json::json!(audio_loudness),
                            );

                            // Convert the Map directly to a Value for serialization
                            let combined_stats = serde_json::Value::Object(flattened_data);

                            // Serialization
                            let ser_data = serde_json::to_vec(&combined_stats)
                                .expect("Failed to serialize for Kafka");

                            // Debug output if enabled
                            if debug_on {
                                let ser_data_str = String::from_utf8_lossy(&ser_data);
                                debug!("MONITOR::PACKET:SERIALIZED_DATA: {}", ser_data_str);
                            }

                            // Check if it's time to send data to Kafka based on the interval
                            if force_send_message
                                || last_kafka_send_time.elapsed().as_millis()
                                    >= args.kafka_interval as u128
                            {
                                // Kafka message production
                                let future = produce_message(
                                    ser_data,
                                    kafka_broker.clone(),
                                    kafka_topic.clone(),
                                    kafka_timeout,
                                    kafka_key.clone(),
                                    current_unix_timestamp_ms().unwrap_or(0) as i64,
                                    producer.clone(),
                                    &admin_client,
                                );

                                // Await the future for sending the message
                                future.await;
                                last_kafka_send_time = Instant::now();
                            }
                        }

                        counter += 1
                    }
                    Err(e) => {
                        error!("Error deserializing message: {:?}", e);
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
            Ok(_) => {
                // No messages were received
                // sleep for a short time to avoid busy waiting
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            Err(e) => {
                error!("Failed to receive message: {:?}", e);
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue; // or handle error as needed
            }
        };
    }

    info!("Finished RsCap monitor");
}
