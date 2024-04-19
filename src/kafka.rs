/*
 * monitor.rs
 *
 * Author: Chris Kennedy (C) 2024
 *
 * License: MIT
 *
 */

use base64::{engine::general_purpose, Engine as _};
use rdkafka::admin::{AdminClient, AdminOptions, NewTopic};
use rdkafka::client::DefaultClientContext;
use rdkafka::config::ClientConfig;
use rdkafka::producer::{FutureProducer, FutureRecord};
use std::fs::File;
use std::io::Write;
use std::time::Instant;
use tokio;
use tokio::time::Duration;
// Include the generated paths for the Cap'n Proto schema

use crate::current_unix_timestamp_ms;
use crate::stream_data::StreamData;
use ahash::AHashMap;
use lazy_static::lazy_static;
use serde_json::{json, Value};
use std::sync::RwLock;

lazy_static! {
    static ref PROBE_DATA: RwLock<AHashMap<String, ProbeData>> = RwLock::new(AHashMap::new());
}

struct ProbeData {
    stream_groupings: AHashMap<u16, StreamGrouping>,
    global_data: serde_json::Map<String, Value>,
}

struct StreamGrouping {
    stream_data_list: Vec<StreamData>,
}

// Callback function to check the status of the sent messages
async fn delivery_report(
    result: Result<(i32, i64), (rdkafka::error::KafkaError, rdkafka::message::OwnedMessage)>,
) {
    match result {
        Ok((partition, offset)) => log::debug!(
            "Message delivered to partition {} at offset {}",
            partition,
            offset
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

pub async fn produce_message(
    data: Vec<u8>,
    kafka_server: String,
    kafka_topic: String,
    kafka_timeout: u64,
    key: String,
    _stream_data_timestamp: i64,
    producer: FutureProducer,
    admin_client: &AdminClient<DefaultClientContext>,
) {
    log::debug!("Service {} sending message", kafka_topic);
    let kafka_topic = kafka_topic.replace(":", "_").replace(".", "_");

    // Metadata fetching is problematic, directly attempt to ensure the topic exists.
    // This code block tries to create the topic if it doesn't already exist, ignoring errors that indicate existence.
    let new_topic = NewTopic::new(&kafka_topic, 1, rdkafka::admin::TopicReplication::Fixed(1));
    let _ = admin_client
        .create_topics(&[new_topic], &AdminOptions::new())
        .await;

    log::debug!(
        "Forwarding message for topic {} to Kafka server {:?}",
        kafka_topic,
        kafka_server
    );

    let record = FutureRecord::to(&kafka_topic).payload(&data).key(&key);
    /* .timestamp(stream_data_timestamp);*/

    let delivery_future = producer
        .send(record, Duration::from_secs(kafka_timeout))
        .await;
    match delivery_future {
        Ok(delivery_result) => delivery_report(Ok(delivery_result)).await,
        Err(e) => log::error!("Failed to send message: {:?}", e),
    }
}

pub async fn send(
    stream_data: &StreamData,
    output_file: String,
    kafka_broker: String,
    kafka_topic: String,
    send_to_kafka: bool,
    kafka_timeout: u64,
    kafka_key: String,
    kafka_interval: u64,
    no_progress: bool,
    output_file_counter: &mut u32,
    last_kafka_send_time: &mut Instant,
    dot_last_file_write: &mut Instant,
) {
    let mut kafka_conf = ClientConfig::new();
    kafka_conf.set("bootstrap.servers", &kafka_broker);
    kafka_conf.set("client.id", "rscap");

    let admin_client: AdminClient<DefaultClientContext> =
        kafka_conf.create().expect("Failed to create admin client");
    let producer: FutureProducer = kafka_conf
        .create()
        .expect("Failed to create Kafka producer");

    let kafka_broker_clone = kafka_broker.clone();
    let kafka_topic_clone = kafka_topic.clone();
    let kafka_timeout_clone = kafka_timeout;
    let kafka_key_clone = kafka_key.clone();
    let producer_clone = producer.clone();

    let mut log_messages = Vec::new();

    let mut base64_image = String::new();
    // change output_file_name_mut to contain an incrementing _00000000.jpg ending
    // use output_file_counter to increment the file name
    // example output_file_name_mut = "output_{:08}.jpg", output_file_counter
    // remove existing .jpg if given first
    let output_file_without_jpg = output_file.replace(".jpg", "");
    if stream_data.packet_len > 0 && stream_data.has_image > 0 {
        let output_file_incremental =
            format!("{}_{:08}.jpg", output_file_without_jpg, output_file_counter);

        *output_file_counter += 1;

        let mut output_file_mut = if !output_file.is_empty() {
            Some(File::create(&output_file_incremental).unwrap())
        } else {
            None
        };
        log::info!(
            "Monitor: [{}] Jpeg image received: {} size {} pts saved to {}",
            stream_data.probe_id,
            stream_data.packet_len,
            stream_data.image_pts,
            output_file_incremental
        );

        // Write to file if output_file is provided
        if let Some(file) = output_file_mut.as_mut() {
            if !no_progress && dot_last_file_write.elapsed().as_secs() > 1 {
                *dot_last_file_write = Instant::now();
                print!("*");
                // flush stdout
                std::io::stdout().flush().unwrap();
            }
            file.write_all(&stream_data.packet).unwrap();
        }

        // Encode the JPEG image as Base64
        base64_image = general_purpose::STANDARD.encode(&stream_data.packet.as_ref());
    }

    let mut probe_id = stream_data.probe_id.clone();
    if probe_id.is_empty() {
        // construct stream.source_ip and stream.source_port with stream.host
        probe_id = format!(
            "{}:{}:{}",
            stream_data.host_name, stream_data.source_ip, stream_data.source_port
        );
    }
    let pid = stream_data.pid;
    {
        let mut probe_data_map = PROBE_DATA.write().unwrap();
        if let Some(probe_data) = probe_data_map.get_mut(&probe_id) {
            let stream_groupings = &mut probe_data.stream_groupings;
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
        } else {
            let mut new_stream_groupings = AHashMap::new();
            let new_grouping = StreamGrouping {
                stream_data_list: vec![stream_data.clone()],
            };
            new_stream_groupings.insert(pid, new_grouping);
            let new_probe_data = ProbeData {
                stream_groupings: new_stream_groupings,
                global_data: serde_json::Map::new(),
            };
            probe_data_map.insert(probe_id.clone(), new_probe_data);
        }
    }

    // Check if it's time to send data to Kafka based on the interval
    if send_to_kafka {
        let mut force_send_message = false;

        // Create a new map to store the averaged probe data
        let mut averaged_probe_data: serde_json::Map<String, serde_json::Value> =
            serde_json::Map::new();

        // Acquire write access to PROBE_DATA
        {
            let mut probe_data_map = PROBE_DATA.write().unwrap();

            // Process each probe's data
            for (_probe_id, probe_data) in probe_data_map.iter_mut() {
                let stream_groupings = &probe_data.stream_groupings;
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
                        total_cc_errors_current += stream_data.current_error_count as u64;
                        source_port = stream_data.source_port as u32;
                        source_ip = stream_data.source_ip.clone();
                        if stream_data.has_image > 0 && stream_data.image_pts > 0 {
                            image_pts = stream_data.image_pts;
                        }
                        if stream_data.log_message != "" {
                            log::info!("Got Log Message: {}", stream_data.log_message);
                            log_messages.push(stream_data.log_message.clone());
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
                            audio_loudness =
                                format!("{}{}", audio_loudness, stream_data.audio_loudness);
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
                let current_timestamp = current_unix_timestamp_ms().unwrap_or(0);

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
                flattened_data.insert("source_port".to_string(), serde_json::json!(source_port));

                if captions != "" {
                    force_send_message = true;
                }
                flattened_data.insert("captions".to_string(), serde_json::json!(captions));

                // Insert the base64_image field into the flattened_data map
                flattened_data.insert("image_pts".to_string(), serde_json::json!(image_pts));
                let base64_image_tag = if !base64_image.is_empty() {
                    log::debug!("Got Image: {} bytes", base64_image.len());
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
                    flattened_data
                        .insert("log_message".to_string(), serde_json::json!(log_message));
                } else {
                    flattened_data.insert("log_message".to_string(), serde_json::json!(""));
                }

                flattened_data.insert("id".to_string(), serde_json::json!(probe_id));
                flattened_data.insert("pid_map".to_string(), serde_json::json!(pid_map));
                flattened_data.insert("scte35".to_string(), serde_json::json!(scte35));
                flattened_data.insert(
                    "audio_loudness".to_string(),
                    serde_json::json!(audio_loudness),
                );

                // Merge the probe-specific flattened data with the global data
                flattened_data.extend(probe_data.global_data.clone());

                // Store the flattened data in the averaged_probe_data map
                averaged_probe_data
                    .insert(probe_id.clone(), serde_json::Value::Object(flattened_data));

                // Clear the global data after processing
                probe_data.global_data.clear();
            }
        }

        // Check if it's time to send data to Kafka based on the interval or if force_send_message is true
        if force_send_message
            || last_kafka_send_time.elapsed().as_millis() >= kafka_interval as u128
        {
            for (_probe_id, probe_data) in averaged_probe_data.iter() {
                let json_data = serde_json::to_value(probe_data)
                    .expect("Failed to serialize probe data for Kafka");

                let ser_data = serde_json::to_vec(&json_data)
                    .expect("Failed to serialize json data for Kafka");

                // Produce the message to Kafka
                let future = produce_message(
                    ser_data,
                    kafka_broker_clone.clone(),
                    kafka_topic_clone.clone(),
                    kafka_timeout_clone,
                    kafka_key_clone.clone(),
                    current_unix_timestamp_ms().unwrap_or(0) as i64,
                    producer_clone.clone(),
                    &admin_client,
                );

                future.await;
            }

            // Update last send time
            *last_kafka_send_time = Instant::now();
        }
    }
}
