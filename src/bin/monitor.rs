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
use log::{debug, error, info};
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
use h264_reader::annexb::AnnexBReader;
use h264_reader::nal::{pps, sei, slice, sps, Nal, RefNal, UnitType};
use h264_reader::push::NalInterest;
use h264_reader::Context;
use lazy_static::lazy_static;
use mpeg2ts_reader::demultiplex;
use rscap::current_unix_timestamp_ms;
use rscap::mpegts;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use tokio::sync::mpsc::{self};
use tokio::task;

lazy_static! {
    static ref STREAM_GROUPINGS: RwLock<AHashMap<u16, StreamGrouping>> =
        RwLock::new(AHashMap::new());
}

struct StreamGrouping {
    stream_data_list: Vec<StreamData>,
}

fn is_cea_608(itu_t_t35_data: &sei::user_data_registered_itu_t_t35::ItuTT35) -> bool {
    // In this example, we check if the ITU-T T.35 data matches the known format for CEA-608.
    // This is a simplified example and might need adjustment based on the actual data format.
    match itu_t_t35_data {
        sei::user_data_registered_itu_t_t35::ItuTT35::UnitedStates => true,
        _ => false,
    }
}

// This function checks if the byte is a standard ASCII character
fn is_standard_ascii(byte: u8) -> bool {
    byte >= 0x20 && byte <= 0x7F
}

// Function to check if the byte pair represents XDS data
fn is_xds(byte1: u8, byte2: u8) -> bool {
    // Implement logic to identify XDS data
    // Placeholder logic: Example only
    byte1 == 0x01 && byte2 >= 0x20 && byte2 <= 0x7F
}

// Function to decode CEA-608 CC1/CC2
fn decode_cea_608_cc1_cc2(byte1: u8, byte2: u8) -> Option<String> {
    decode_character(byte1, byte2)
    // The above line replaces the previous implementation and uses decode_character
    // to handle both ASCII and control codes.
}

fn decode_cea_608_xds(byte1: u8, byte2: u8) -> Option<String> {
    if is_xds(byte1, byte2) {
        Some(format!("XDS: {:02X} {:02X}", byte1, byte2))
    } else {
        None
    }
}

// Decode CEA-608 characters, including control codes
fn decode_character(byte1: u8, byte2: u8) -> Option<String> {
    debug!("Decoding: {:02X} {:02X}", byte1, byte2); // Debugging

    // Handle standard ASCII characters
    if is_standard_ascii(byte1) && is_standard_ascii(byte2) {
        return Some(format!("{}{}", byte1 as char, byte2 as char));
    }

    // Handle special control characters (Example)
    // This is a simplified version, actual implementation may vary based on control characters
    match (byte1, byte2) {
        (0x14, 0x2C) => Some(String::from("[Clear Caption]")),
        (0x14, 0x20) => Some(String::from("[Roll-Up Caption]")),
        // Add more control character handling here
        _ => {
            error!("Unhandled control character: {:02X} {:02X}", byte1, byte2); // Debugging
            None
        }
    }
}

// Simplified CEA-608 decoding function
// Main CEA-608 decoding function
fn decode_cea_608(data: &[u8]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut captions_cc1 = Vec::new();
    let mut captions_cc2 = Vec::new();
    let mut xds_data = Vec::new();

    for chunk in data.chunks(3) {
        if chunk.len() == 3 {
            match chunk[0] {
                0x04 => {
                    if let Some(decoded) = decode_cea_608_cc1_cc2(chunk[1], chunk[2]) {
                        captions_cc1.push(decoded);
                    } else if let Some(decoded) = decode_cea_608_xds(chunk[1], chunk[2]) {
                        xds_data.push(decoded);
                    }
                }
                0x05 => {
                    if let Some(decoded) = decode_cea_608_cc1_cc2(chunk[1], chunk[2]) {
                        captions_cc2.push(decoded);
                    }
                }
                _ => debug!("Unknown caption channel: {:02X}", chunk[0]),
            }
        }
    }

    (captions_cc1, captions_cc2, xds_data)
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
        Err(e) => println!("Failed to send message: {:?}", e),
    }
}

#[derive(Parser, Debug)]
#[clap(
    author = "Chris Kennedy",
    version = "0.5.8",
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
    #[clap(long, env = "KAFKA_TIMEOUT", default_value_t = 10)]
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

    /// MPSC Channel Size for Decoder
    #[clap(long, env = "DECODER_CHANNEL_SIZE", default_value_t = 1_000)]
    decoder_channel_size: usize,

    /// Demuxer Channel size
    #[clap(long, env = "DEMUXER_CHANNEL_SIZE", default_value_t = 1_000)]
    demuxer_channel_size: usize,

    /// Decode Video
    #[clap(long, env = "DECODE_VIDEO", default_value_t = false)]
    decode_video: bool,

    /// Decode Video Batch Size
    #[clap(long, env = "DECODE_VIDEO_BATCH_SIZE", default_value_t = 100)]
    decode_video_batch_size: usize,

    /// Debug NALs
    #[clap(long, env = "DEBUG_NALS", default_value_t = false)]
    debug_nals: bool,

    /// List of NAL types to debug, comma separated: all, sps, pps, pic_timing, sei, slice, user_data_registered_itu_tt35, user_data_unregistered, buffering_period, unknown
    #[clap(
        long,
        env = "DEBUG_NAL_TYPES",
        default_value = "",
        help = "List of NAL types to debug, comma separated: all, sps, pps, pic_timing, sei, slice, user_data_registered_itu_tt35, user_data_unregistered, buffering_period, unknown"
    )]
    debug_nal_types: String,

    // Parse short NALs that are 0x000001
    #[clap(long, env = "PARSE_SHORT_NALS", default_value_t = false)]
    parse_short_nals: bool,

    // MpegTS Reader use
    #[clap(long, env = "MPEGTS_READER", default_value_t = false)]
    mpegts_reader: bool,

    // Add the new argument for Kafka interval
    /// Kafka sending interval in milliseconds
    #[clap(long, env = "KAFKA_INTERVAL", default_value_t = 1000)]
    kafka_interval: u64,
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

    let running = Arc::new(AtomicBool::new(true));
    let running_decoder = running.clone();
    let running_demuxer = running.clone();

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

    // Initialize logging
    let _ = env_logger::try_init();
    let mut ctx = Context::default();
    let mut scratch = Vec::new();
    // Use the `move` keyword to move ownership of `ctx` and `scratch` into the closure
    let mut annexb_reader = AnnexBReader::accumulate(move |nal: RefNal<'_>| {
        if !nal.is_complete() {
            return NalInterest::Buffer;
        }
        let hdr = match nal.header() {
            Ok(h) => h,
            Err(e) => {
                // check if we are in debug mode for nals, else check if this is a ForbiddenZeroBit error, which we ignore
                let e_str = format!("{:?}", e);
                if !args.debug_nals && e_str == "ForbiddenZeroBit" {
                    // ignore forbidden zero bit error unless we are in debug mode
                } else {
                    // show nal contents
                    debug!("---\n{:?}\n---", nal);
                    error!("Failed to parse NAL header: {:?}", e);
                }
                return NalInterest::Buffer;
            }
        };
        match hdr.nal_unit_type() {
            UnitType::SeqParameterSet => {
                if let Ok(sps) = sps::SeqParameterSet::from_bits(nal.rbsp_bits()) {
                    // check if debug_nal_types has sps
                    if args.debug_nal_types.contains(&"sps".to_string())
                        || args.debug_nal_types.contains(&"all".to_string())
                    {
                        println!("Found SPS: {:?}", sps);
                    }
                    ctx.put_seq_param_set(sps);
                }
            }
            UnitType::PicParameterSet => {
                if let Ok(pps) = pps::PicParameterSet::from_bits(&ctx, nal.rbsp_bits()) {
                    // check if debug_nal_types has pps
                    if args.debug_nal_types.contains(&"pps".to_string())
                        || args.debug_nal_types.contains(&"all".to_string())
                    {
                        println!("Found PPS: {:?}", pps);
                    }
                    ctx.put_pic_param_set(pps);
                }
            }
            UnitType::SEI => {
                let mut r = sei::SeiReader::from_rbsp_bytes(nal.rbsp_bytes(), &mut scratch);
                while let Ok(Some(msg)) = r.next() {
                    match msg.payload_type {
                        sei::HeaderType::PicTiming => {
                            let sps = match ctx.sps().next() {
                                Some(s) => s,
                                None => continue,
                            };
                            let pic_timing = sei::pic_timing::PicTiming::read(sps, &msg);
                            match pic_timing {
                                Ok(pic_timing_data) => {
                                    // Check if debug_nal_types has pic_timing or all
                                    if args.debug_nal_types.contains(&"pic_timing".to_string())
                                        || args.debug_nal_types.contains(&"all".to_string())
                                    {
                                        println!("Found PicTiming: {:?}", pic_timing_data);
                                    }
                                }
                                Err(e) => {
                                    error!("Error parsing PicTiming SEI: {:?}", e);
                                }
                            }
                        }
                        h264_reader::nal::sei::HeaderType::BufferingPeriod => {
                            let sps = match ctx.sps().next() {
                                Some(s) => s,
                                None => continue,
                            };
                            let buffering_period =
                                sei::buffering_period::BufferingPeriod::read(&ctx, &msg);
                            // check if debug_nal_types has buffering_period
                            if args
                                .debug_nal_types
                                .contains(&"buffering_period".to_string())
                                || args.debug_nal_types.contains(&"all".to_string())
                            {
                                println!(
                                    "Found BufferingPeriod: {:?} Payload: [{:?}] - {:?}",
                                    buffering_period, msg.payload, sps
                                );
                            }
                        }
                        h264_reader::nal::sei::HeaderType::UserDataRegisteredItuTT35 => {
                            match sei::user_data_registered_itu_t_t35::ItuTT35::read(&msg) {
                                Ok((itu_t_t35_data, remaining_data)) => {
                                    if args
                                        .debug_nal_types
                                        .contains(&"user_data_registered_itu_tt35".to_string())
                                        || args.debug_nal_types.contains(&"all".to_string())
                                    {
                                        println!("Found UserDataRegisteredItuTT35: {:?}, Remaining Data: {:?}", itu_t_t35_data, remaining_data);
                                    }
                                    if is_cea_608(&itu_t_t35_data) {
                                        let (captions_cc1, captions_cc2, xds_data) =
                                            decode_cea_608(remaining_data);
                                        debug!(
                                            "CEA-608 Data: {:?} cc1: {:?} cc2: {:?} xds: {:?}",
                                            itu_t_t35_data, captions_cc1, captions_cc2, xds_data
                                        );
                                        if !captions_cc1.is_empty() {
                                            debug!("CEA-608 CC1 Captions: {:?}", captions_cc1);
                                        }
                                        if !captions_cc2.is_empty() {
                                            debug!("CEA-608 CC2 Captions: {:?}", captions_cc2);
                                        }
                                        if !xds_data.is_empty() {
                                            debug!("CEA-608 XDS Data: {:?}", xds_data);
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!("Error parsing ITU T.35 data: {:?}", e);
                                }
                            }
                        }
                        h264_reader::nal::sei::HeaderType::UserDataUnregistered => {
                            // Check if debug_nal_types has user_data_unregistered or all
                            if args
                                .debug_nal_types
                                .contains(&"user_data_unregistered".to_string())
                                || args.debug_nal_types.contains(&"all".to_string())
                            {
                                println!(
                                    "Found SEI type UserDataUnregistered {:?} payload: [{:?}]",
                                    msg.payload_type, msg.payload
                                );
                            }
                        }
                        _ => {
                            // check if debug_nal_types has sei
                            if args.debug_nal_types.contains(&"sei".to_string())
                                || args.debug_nal_types.contains(&"all".to_string())
                            {
                                println!(
                                    "Unknown Found SEI type {:?} payload: [{:?}]",
                                    msg.payload_type, msg.payload
                                );
                            }
                        }
                    }
                }
            }
            UnitType::SliceLayerWithoutPartitioningIdr
            | UnitType::SliceLayerWithoutPartitioningNonIdr => {
                let msg = slice::SliceHeader::from_bits(&ctx, &mut nal.rbsp_bits(), hdr);
                // check if debug_nal_types has slice
                if args.debug_nal_types.contains(&"slice".to_string())
                    || args.debug_nal_types.contains(&"all".to_string())
                {
                    println!("Found NAL Slice: {:?}", msg);
                }
            }
            _ => {
                // check if debug_nal_types has nal
                if args.debug_nal_types.contains(&"unknown".to_string())
                    || args.debug_nal_types.contains(&"all".to_string())
                {
                    println!("Found Unknown NAL: {:?}", nal);
                }
            }
        }
        NalInterest::Buffer
    });

    // Setup demuxer async processing thread
    let (dmtx, mut dmrx) = mpsc::channel::<Vec<u8>>(args.demuxer_channel_size);

    // Setup asynchronous demuxer processing thread
    let (sync_dmtx, mut sync_dmrx) = mpsc::channel::<Vec<u8>>(args.demuxer_channel_size);

    // Running a synchronous task in the background
    let running_demuxer_clone = running_demuxer.clone();
    task::spawn_blocking(move || {
        let mut demux_ctx = mpegts::DumpDemuxContext::new();
        let mut demux = demultiplex::Demultiplex::new(&mut demux_ctx);
        let mut demux_buf = [0u8; 1880 * 1024];
        let mut buf_end = 0;

        info!("Running Demuxer clone thread started");

        while running_demuxer_clone.load(Ordering::SeqCst) {
            match sync_dmrx.blocking_recv() {
                Some(packet) => {
                    let packet_len = packet.len();
                    let space_left = demux_buf.len() - buf_end;

                    if space_left < packet_len {
                        buf_end = 0; // Reset buffer on overflow
                    }

                    demux_buf[buf_end..buf_end + packet_len].copy_from_slice(&packet);
                    buf_end += packet_len;

                    /*let packet_arc = Arc::new(packet);
                    hexdump(&packet_arc, 0, packet_len);*/
                    demux.push(&mut demux_ctx, &demux_buf[0..buf_end]);
                    // Additional processing as required
                }
                None => {
                    // Handle error or shutdown
                    break;
                }
            }
        }
    });

    // Initialize the mpegts demuxer thread using Tokio
    let demuxer_thread = tokio::spawn(async move {
        info!("Base Demuxer thread started");
        loop {
            if !running_demuxer.load(Ordering::SeqCst) {
                debug!("Demuxer thread received stop signal.");
                break;
            }

            if !args.mpegts_reader {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }

            if let Some(packet) = dmrx.recv().await {
                // Send packet data to the synchronous processing thread
                //info!("Demuxer thread received packet of size: {}", packet.len());
                sync_dmtx.send(packet).await.unwrap();
            }
        }
    });

    // Initialize the video processor
    // Setup channel for passing data between threads
    let (dtx, mut drx) = mpsc::channel::<Vec<StreamData>>(args.decoder_channel_size);
    // Spawn a new thread for Decoder communication
    let decoder_thread = tokio::spawn(async move {
        info!("Decoder thread started");
        loop {
            if !running_decoder.load(Ordering::SeqCst) {
                debug!("Decoder thread received stop signal.");
                break;
            }

            if !args.mpegts_reader && !args.decode_video {
                // Sleep for a short duration to prevent a tight loop
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }

            // Use tokio::select to simultaneously wait for a new batch or a stop signal
            tokio::select! {
                Some(mut batch) = drx.recv() => {
                    debug!("Processing {} video packets in decoder thread", batch.len());
                    for stream_data in &batch {
                        // packet is a subset of the original packet, starting at the payload
                        let packet_start = stream_data.packet_start;
                        let packet_end = stream_data.packet_start + stream_data.packet_len;

                        if packet_end - packet_start > args.packet_size {
                            error!("NAL Parser: Packet size {} is larger than packet buffer size {}. Skipping packet.",
                                packet_end - packet_start, args.packet_size);
                            continue;
                        }

                        // check if packet_start + 4 is less than packet_end
                        if packet_start + 4 >= packet_end {
                            error!("NAL Parser: Packet size {} {} - {} is less than 4 bytes. Skipping packet.",
                                packet_end - packet_start, packet_start, packet_end);
                            continue;
                        }

                        if args.mpegts_reader {
                            // Send packet data to the synchronous processing thread
                            dmtx.send(stream_data.packet[packet_start..packet_end].to_vec()).await.unwrap();

                            // check if we are decoding video
                            if !args.decode_video {
                                continue;
                            }
                        }

                        // Skip MPEG-TS header and adaptation field
                        let header_len = 4;
                        let adaptation_field_control = (stream_data.packet[packet_start + 3] & 0b00110000) >> 4;

                        if adaptation_field_control == 0b10 {
                            continue; // Skip packets with only adaptation field (no payload)
                        }

                        let payload_start = if adaptation_field_control != 0b01 {
                            header_len + 1 + stream_data.packet[packet_start + 4] as usize
                        } else {
                            header_len
                        };

                        // confirm payload_start is sane
                        if payload_start >= packet_end || packet_end - payload_start < 4 {
                            debug!("NAL Parser: Payload start {} is invalid with packet_start as {} and packet_end as {}. Skipping packet.",
                                payload_start, packet_start, packet_end);
                            //hexdump(&stream_data.packet, packet_start, packet_end - packet_start);
                            continue;
                        } else {
                            debug!("NAL Parser: Payload start {} is valid with packet_start as {} and packet_end as {}.",
                                payload_start, packet_start, packet_end);
                        }

                        // Process payload, skipping padding bytes
                        let mut pos = payload_start;
                        while pos + 4 < packet_end {
                            if args.parse_short_nals && stream_data.packet[pos..pos + 3] == [0x00, 0x00, 0x01] {
                                let nal_start = pos;
                                pos += 3; // Move past the short start code

                                // Search for the next start code
                                while pos + 4 <= packet_end &&
                                      stream_data.packet[pos..pos + 4] != [0x00, 0x00, 0x00, 0x01] {
                                    // Check for short start code, 0xff padding, or 0x00000000 sequence
                                    if stream_data.packet[pos..pos + 3] == [0x00, 0x00, 0x01] && pos > nal_start + 3 {
                                        // Found a short start code, so back up and process the NAL unit
                                        break;
                                    } else if stream_data.packet[pos + 1] == 0xff && pos > nal_start + 3 {
                                        // check for 0xff padding and that we are at least 2 bytes into the nal
                                        break;
                                    } else if stream_data.packet[pos..pos + 3] == [0x00, 0x00, 0x00] && pos > nal_start + 3 {
                                        // check for 0x00 0x00 0x00 0x00 sequence to stop at
                                        break;
                                    }
                                    pos += 1;
                                }

                                // check if we only have 4 bytes left in the packet, if so then collect them too
                                if pos + 4 >= packet_end {
                                    while pos < packet_end {
                                        if stream_data.packet[pos..pos + 1] == [0xff] {
                                            // check for 0xff padding and that we are at least 2 bytes into the nal
                                            break;
                                        } else if pos + 2 < packet_end && stream_data.packet[pos..pos + 2] == [0x00, 0x00] {
                                            // check for 0x00 0x00 sequence to stop at
                                            break;
                                        }
                                        pos += 1;
                                    }
                                }

                                let nal_end = pos; // End of NAL unit found or end of packet
                                if nal_end - nal_start > 3 { // Threshold for significant NAL unit size
                                    let nal_unit = &stream_data.packet[nal_start..nal_end];

                                    // Debug print the NAL unit
                                    if args.debug_nals {
                                        let packet_len = nal_end - nal_start;
                                        info!("Extracted {} byte Short NAL Unit from packet range {}-{}:", packet_len, nal_start, nal_end);
                                        let nal_unit_arc = Arc::new(nal_unit.to_vec());
                                        hexdump(&nal_unit_arc, 0, packet_len);
                                    }

                                    // Process the NAL unit
                                    annexb_reader.push(nal_unit);
                                    annexb_reader.reset();
                                }
                            } else if pos + 4 < packet_end && stream_data.packet[pos..pos + 4] == [0x00, 0x00, 0x00, 0x01] {
                                let nal_start = pos;
                                pos += 4; // Move past the long start code

                                // Search for the next start code
                                while pos + 4 <= packet_end &&
                                      stream_data.packet[pos..pos + 4] != [0x00, 0x00, 0x00, 0x01] {
                                    // Check for short start code
                                    if stream_data.packet[pos..pos + 3] == [0x00, 0x00, 0x01] && pos > nal_start + 3 {
                                        // Found a short start code, so back up and process the NAL unit
                                        break;
                                    } else if stream_data.packet[pos + 1] == 0xff && pos > nal_start + 3 {
                                        // check for 0xff padding and that we are at least 2 bytes into the nal
                                        break;
                                    } else if stream_data.packet[pos..pos + 3] == [0x00, 0x00, 0x00] && pos > nal_start + 3 {
                                        // check for 0x00 0x00 0x00 0x00 sequence to stop at
                                        break;
                                    }
                                    pos += 1;
                                }

                                // check if we only have 4 bytes left in the packet, if so then collect them too
                                if pos + 4 >= packet_end {
                                    while pos < packet_end {
                                        if stream_data.packet[pos..pos + 1] == [0xff] {
                                            // check for 0xff padding and that we are at least 2 bytes into the nal
                                            break;
                                        } else if pos + 2 < packet_end && stream_data.packet[pos..pos + 2] == [0x00, 0x00] {
                                            // check for 0x00 0x00 sequence to stop at
                                            break;
                                        }
                                        pos += 1;
                                    }
                                }

                                let nal_end = pos; // End of NAL unit found or end of packet
                                if nal_end - nal_start > 3 { // Threshold for significant NAL unit size
                                    let nal_unit = &stream_data.packet[nal_start..nal_end];

                                    // Debug print the NAL unit
                                    if args.debug_nals {
                                        let packet_len = nal_end - nal_start;
                                        let nal_unit_arc = Arc::new(nal_unit.to_vec());
                                        hexdump(&nal_unit_arc, 0, packet_len);
                                        info!("Extracted {} byte Long NAL Unit from packet range {}-{}:", packet_len, nal_start, nal_end);
                                    }

                                    // Process the NAL unit
                                    annexb_reader.push(nal_unit);
                                    annexb_reader.reset();
                                }
                            } else {
                                pos += 1; // Move to the next byte if no start code found
                            }
                        }
                    }
                    // Clear the batch after processing
                    batch.clear();
                }
                _ = tokio::time::sleep(Duration::from_millis(10)), if !running_decoder.load(Ordering::SeqCst) => {
                    // This branch allows checking the running flag regularly
                    info!("Decoder thread received stop signal.");
                    break;
                }
            }
        }
    });

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

    let mut video_batch = Vec::new();

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

    loop {
        // check for packet count
        if packet_count > 0 && counter >= packet_count {
            break;
        }

        // Now, receive the data message
        let packet_msg = zmq_sub
            .recv_multipart(0)
            .expect("Failed to receive header message");

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
                log::debug!("MONITOR::PACKET:RECEIVE[{}] pid: {} stream_type: {} bitrate: {} bitrate_max: {} bitrate_min: {} bitrate_avg: {} iat: {} iat_max: {} iat_min: {} iat_avg: {} errors: {} continuity_counter: {} timestamp: {}",
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

                    if args.decode_video || args.mpegts_reader {
                        if video_batch.len() >= args.decode_video_batch_size {
                            dtx.send(video_batch).await.unwrap(); // Clone if necessary
                            video_batch = Vec::new();
                        } else {
                            let mut stream_data_clone = stream_data.clone();
                            stream_data_clone.packet_start = 0;
                            stream_data_clone.packet_len = data_msg.len();
                            stream_data_clone.packet = Arc::new(data_msg.to_vec());
                            video_batch.push(stream_data_clone);
                        }
                    }

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
                        log::debug!(
                            "Monitor: Jpeg image received: {} size {} pts",
                            data_msg.len(),
                            stream_data.image_pts
                        );
                        let output_file_incremental =
                            format!("{}_{:08}.jpg", output_file_without_jpg, output_file_counter);

                        let mut output_file_mut = if !output_file.is_empty() {
                            Some(File::create(&output_file_incremental).unwrap())
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

                        // Encode the JPEG image as Base64
                        base64_image = general_purpose::STANDARD.encode(&data_msg);
                    }
                }

                let pid = stream_data.pid;
                {
                    let mut stream_groupings = STREAM_GROUPINGS.write().unwrap();
                    if let Some(grouping) = stream_groupings.get_mut(&pid) {
                        // Update the existing StreamData instance in the grouping
                        let last_stream_data = grouping.stream_data_list.last_mut().unwrap();
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

                    // Process each stream to accumulate averages
                    for (_, grouping) in stream_groupings.iter() {
                        for stream_data in &grouping.stream_data_list {
                            total_bitrate_avg += stream_data.bitrate_avg as u64;
                            total_iat_avg += stream_data.capture_iat;
                            total_iat_max += stream_data.capture_iat_max;
                            total_cc_errors += stream_data.error_count as u64;
                            total_cc_errors_current += stream_data.current_error_count as u64;
                            source_port = stream_data.source_port as u32;
                            source_ip = stream_data.source_ip.clone();
                            if stream_data.has_image > 0 && stream_data.image_pts > 0 {
                                image_pts = stream_data.image_pts;
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
                    flattened_data.insert("source_ip".to_string(), serde_json::json!(source_ip));
                    flattened_data
                        .insert("source_port".to_string(), serde_json::json!(source_port));

                    // Insert the base64_image field into the flattened_data map
                    flattened_data.insert("image_pts".to_string(), serde_json::json!(image_pts));
                    let base64_image_tag = if base64_image != "" {
                        format!("data:image/jpeg;base64,{}", base64_image)
                    } else {
                        "".to_string()
                    };
                    flattened_data.insert(
                        "base64_image".to_string(),
                        serde_json::json!(base64_image_tag),
                    );

                    // Convert the Map directly to a Value for serialization
                    let combined_stats = serde_json::Value::Object(flattened_data);

                    // Serialization
                    let ser_data =
                        serde_json::to_vec(&combined_stats).expect("Failed to serialize for Kafka");

                    // Debug output if enabled
                    if debug_on {
                        let ser_data_str = String::from_utf8_lossy(&ser_data);
                        debug!("MONITOR::PACKET:SERIALIZED_DATA: {}", ser_data_str);
                    }

                    // Check if it's time to send data to Kafka based on the interval
                    if base64_image_tag != ""
                        || last_kafka_send_time.elapsed().as_millis() >= args.kafka_interval as u128
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
            }
            Err(e) => {
                error!("Error deserializing message: {:?}", e);
            }
        }

        if !no_progress && dot_last_sent_ts.elapsed().as_secs() > 1 {
            dot_last_sent_ts = Instant::now();
            print!(".");
            // flush stdout
            std::io::stdout().flush().unwrap();
        }

        counter += 1;
    }

    demuxer_thread.await.unwrap();
    decoder_thread.await.unwrap();

    info!("Finished RsCap monitor");
}
