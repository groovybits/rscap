/*
 * stream_data.rs
 *
 * Data structure for the stream data
*/

use crate::current_unix_timestamp_ms;
use ahash::AHashMap;
use crc::{Crc, CRC_32_ISO_HDLC};
#[cfg(feature = "gst")]
use gst_app::{AppSink, AppSrc};
#[cfg(feature = "gst")]
use gstreamer as gst;
#[cfg(feature = "gst")]
use gstreamer::parse;
#[cfg(feature = "gst")]
use gstreamer::prelude::*;
#[cfg(feature = "gst")]
use gstreamer_app as gst_app;
#[cfg(feature = "gst")]
use gstreamer_video::video_meta::VideoCaptionMeta;
#[cfg(feature = "gst")]
use gstreamer_video::VideoCaptionType;
#[cfg(feature = "gst")]
use gstreamer_video::VideoInfo;
#[cfg(feature = "gst")]
use image::buffer::ConvertBuffer;
#[cfg(feature = "gst")]
use image::Luma;
#[cfg(feature = "gst")]
use image::{ImageBuffer, Rgb};
use lazy_static::lazy_static;
use log::{debug, error, info};
#[cfg(feature = "gst")]
use opencv::img_hash::PHash;
#[cfg(feature = "gst")]
use opencv::{core::Mat, prelude::*};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
#[cfg(feature = "gst")]
use std::fs::OpenOptions;
#[cfg(feature = "gst")]
use std::io;
#[cfg(feature = "gst")]
use std::io::Write;
#[cfg(feature = "gst")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{fmt, sync::Arc, sync::Mutex};
#[cfg(feature = "gst")]
use tokio::sync::mpsc;

const IAT_CAPTURE_WINDOW_SIZE: usize = 100;

lazy_static! {
    static ref PID_MAP: RwLock<AHashMap<u16, Arc<StreamData>>> = RwLock::new(AHashMap::new());
    static ref IAT_CAPTURE_WINDOW: Mutex<VecDeque<u64>> =
        Mutex::new(VecDeque::with_capacity(IAT_CAPTURE_WINDOW_SIZE));
    static ref IAT_CAPTURE_PEAK: Mutex<u64> = Mutex::new(0);
}

#[derive(Clone, Debug)]
pub struct ImageData {
    pub image: Vec<u8>,
    pub pts: u64,
    pub duplicates: u64,
    pub hash: u64,
    pub hamming: f64,
}

#[cfg(feature = "gst")]
fn create_pipeline(desc: &str) -> Result<gst::Pipeline, anyhow::Error> {
    let pipeline = parse::launch(desc)?
        .downcast::<gst::Pipeline>()
        .expect("Expected a gst::Pipeline");
    Ok(pipeline)
}

#[cfg(feature = "gst")]
pub fn initialize_pipeline(
    input_codec: &str,
    height: u32,
    buffer_count: u32,
    scale: bool,
    framerate: &str,
    extract_images: bool,
) -> Result<(gst::Pipeline, gst_app::AppSrc, gst_app::AppSink), anyhow::Error> {
    gst::init()?;

    let width = (((height as f32 * 16.0 / 9.0) / 16.0).round() * 16.0) as u32;
    let scale_string = if scale {
        format!("! videoscale ! video/x-raw,width={width},height={height}")
    } else {
        String::new()
    };

    let stream_type_number = if input_codec == "mpeg2" {
        0x02
    } else if input_codec == "h264" {
        0x1B
    } else if input_codec == "h265" {
        0x24
    } else {
        return Err(anyhow::anyhow!("Unsupported video codec {}", input_codec));
    };

    // Create a pipeline to extract video frames
    let pipeline = match stream_type_number {
        0x02 => create_pipeline(
           &format!("appsrc name=src ! tsdemux ! \
               mpeg2dec ! videorate ! video/x-raw,framerate={} ! videoconvert ! video/x-raw,format=RGB {} ! \
               appsink name=sink", framerate, scale_string),
        )?,
        0x1B => create_pipeline(
              &format!("appsrc name=src ! tsdemux ! \
                  h264parse ! avdec_h264 ! videorate ! video/x-raw,framerate={} ! \
                  videoconvert ! video/x-raw,format=RGB {} ! appsink name=sink", framerate, scale_string),
        )?,
        0x24 => create_pipeline(
                &format!("appsrc name=src ! tsdemux ! \
                    h265parse ! avdec_h265 ! videorate ! video/x-raw,framerate={} ! \
                    videoconvert ! video/x-raw,format=RGB {} ! appsink name=sink", framerate, scale_string),
        )?,
        _ => {
            return Err(anyhow::anyhow!(
                "Unsupported video stream type {}",
                stream_type_number
            ))
        }
    };

    let appsrc = pipeline
        .by_name("src")
        .ok_or_else(|| anyhow::anyhow!("Failed to get appsrc"))?
        .downcast::<gst_app::AppSrc>()
        .map_err(|_| anyhow::anyhow!("AppSrc casting failed"))?;
    let appsink = pipeline
        .by_name("sink")
        .ok_or_else(|| anyhow::anyhow!("Failed to get appsink"))?
        .downcast::<gst_app::AppSink>()
        .map_err(|_| anyhow::anyhow!("AppSink casting failed"))?;

    // Set properties on AppSrc to control buffer build-up
    appsrc.set_property("max-bytes", buffer_count as u64 * 1024 * 1024);
    appsrc.set_property("block", true);
    appsrc.set_property("emit-signals", true);

    appsink.set_property("max-buffers", buffer_count);
    appsink.set_property("drop", true);

    Ok((pipeline, appsrc, appsink))
}

#[cfg(feature = "gst")]
pub fn process_video_packets(
    appsrc: gst_app::AppSrc,
    mut video_packet_receiver: mpsc::Receiver<Vec<u8>>,
    running: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        let mut errors = 0;
        while let Some(packet) = video_packet_receiver.recv().await {
            if !running.load(Ordering::SeqCst) {
                break;
            }
            let buffer = gst::Buffer::from_slice(packet);

            // Push buffer only if not full
            if let Err(err) = appsrc.push_buffer(buffer) {
                eprintln!("Buffer full with {} errors, dropping packet: {}", errors, err);
                errors += 1;
                if errors > 1000 {
                    break;
                }
            } else {
                errors = 0;
            }
        }
    });
}

#[cfg(feature = "gst")]
pub fn pull_images(
    appsink: AppSink,
    image_sender: mpsc::Sender<(Vec<u8>, u64, u64, u64, f64)>,
    save_images: bool,
    sample_interval: u64,
    image_height: u32,
    filmstrip_length: usize,
    jpeg_quality: u8,
    frame_increment: u8,
    get_captions: bool,
    running: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        let mut frame_count = 0;
        let mut last_processed_pts = 0;
        let mut filmstrip_images = Vec::new();
        let save_captions = false;
        let mut prev_hash = None;
        let mut cur_hash = None;
        let mut hamming: f64 = 0.0;
        let mut image_same = 0;
        let frozen_frame_threshold = 30; // Number of frames to detect frozen picture

        while running.load(Ordering::SeqCst) {
            let sample = appsink.try_pull_sample(gst::ClockTime::ZERO);
            if let Some(sample) = sample {
                let buffer = sample.buffer().unwrap();
                let pts = buffer
                    .pts()
                    .map_or(last_processed_pts, |pts| pts.nseconds());

                if last_processed_pts == 0 || pts >= last_processed_pts + sample_interval {
                    last_processed_pts = pts;

                    let map = buffer.map_readable().unwrap();
                    let data = map.as_slice().to_vec();
                    drop(map);

                    // Check if the data length matches the expected dimensions for RGB format
                    let info = VideoInfo::from_caps(&sample.caps().expect("Sample without caps"))
                        .expect("Failed to parse caps");
                    let width = info.width() as usize;
                    let height = info.height() as usize;

                    // Check if height is the same as our image_height, if not, scale it to image_height while keeping the aspect ratio
                    let (scaled_width, scaled_height, scaled_data) =
                        if height != image_height as usize {
                            let aspect_ratio = width as f32 / height as f32;
                            let scaled_height = image_height as usize;
                            let scaled_width = (scaled_height as f32 * aspect_ratio) as usize;

                            let scaled = image::imageops::resize(
                                &image::ImageBuffer::<image::Rgb<u8>, _>::from_raw(
                                    width as u32,
                                    height as u32,
                                    data,
                                )
                                .unwrap(),
                                scaled_width as u32,
                                scaled_height as u32,
                                image::imageops::FilterType::Triangle,
                            );

                            (scaled_width, scaled_height, scaled.into_raw())
                        } else {
                            (width, height, data)
                        };

                    let expected_length = scaled_width * scaled_height * 3; // 3 bytes per pixel for RGB

                    log::debug!(
                        "pull_images: Image scaled to {}x{}",
                        scaled_width,
                        scaled_height
                    );

                    if scaled_data.len() == expected_length {
                        let image = ImageBuffer::<Rgb<u8>, _>::from_raw(
                            scaled_width as u32,
                            scaled_height as u32,
                            scaled_data,
                        )
                        .expect("Failed to create ImageBuffer");

                        // Convert the RGB image to grayscale (GRAY8)
                        let gray_image: ImageBuffer<Luma<u8>, Vec<u8>> = image.convert();

                        // Create a perceptual hash from the grayscale image using the opencv img_hash module
                        let mut current_hash = Mat::default();
                        {
                            let (_width, height) = gray_image.dimensions();
                            let data = gray_image.into_raw();
                            let gray_mat_1d = Mat::from_slice(&data).unwrap();
                            let gray_mat = gray_mat_1d.reshape(1, height as i32).unwrap();
                            let mut hasher = PHash::create().unwrap();
                            hasher.compute(&gray_mat, &mut current_hash).unwrap();
                        }

                        // Compare with the previous hash value
                        if let Some(ref prev_hash) = prev_hash {
                            let hasher = PHash::create().unwrap();
                            let hamming_distance =
                                hasher.compare(prev_hash, &current_hash).unwrap();
                            hamming = hamming_distance;
                            cur_hash = Some(current_hash.clone());
                            log::debug!("Hamming Distance: {}", hamming_distance);

                            // Check if the current frame is the same as the previous frame
                            if hamming_distance == 0.0 {
                                image_same += 1;

                                // Check if the number of consecutive duplicate frames exceeds the threshold
                                if image_same >= frozen_frame_threshold {
                                    log::warn!(
                                        "Frozen frame detected! Consecutive duplicate frames: {}",
                                        image_same
                                    );
                                    // Perform any necessary actions for frozen frames
                                }
                            } else {
                                // Reset the duplicate frame counter if the frames are different
                                image_same = 0;
                            }

                            // Trigger alerts or perform actions based on the hamming distance
                            if hamming_distance > 10.0 {
                                log::debug!("Significant perceptual difference detected!");
                            }
                        }

                        // Update the previous hash for the next iteration
                        prev_hash = Some(current_hash);

                        // Save the image as a JPEG with timecode
                        if save_images {
                            let filename = format!("images/frame_{:04}.jpg", frame_count);
                            let mut jpeg_data = Vec::new();
                            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                                &mut jpeg_data,
                                jpeg_quality,
                            );
                            encoder
                                .encode_image(&image)
                                .expect("Failed to encode image to JPEG");
                            std::fs::write(&filename, &jpeg_data)
                                .expect("Failed to save JPEG image");
                        }
                        frame_count += 1;

                        print!("*");
                        // flush stdout
                        io::stdout().flush().unwrap();

                        filmstrip_images.push(image);

                        if filmstrip_length <= 1 || filmstrip_images.len() >= filmstrip_length {
                            // Create a new image buffer for the filmstrip
                            let filmstrip_width = (scaled_width * filmstrip_length) as u32;
                            let mut filmstrip = ImageBuffer::<Rgb<u8>, Vec<u8>>::new(
                                filmstrip_width,
                                scaled_height as u32,
                            );

                            // Concatenate the images into the filmstrip
                            for (i, img) in filmstrip_images.iter().enumerate() {
                                let x_offset = i as u32 * scaled_width as u32;
                                image::imageops::overlay(&mut filmstrip, img, x_offset.into(), 0);
                            }

                            // Encode the filmstrip to a JPEG vector
                            let mut jpeg_data = Vec::new();
                            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                                &mut jpeg_data,
                                jpeg_quality,
                            );
                            encoder
                                .encode_image(&filmstrip)
                                .expect("JPEG encoding failed");

                            // Send the filmstrip over the channel
                            let hash_value = if let Some(ref hash_mat) = cur_hash {
                                let hash_bytes: [u8; 8] = (0..hash_mat.cols())
                                    .map(|i| *hash_mat.at::<u8>(i).unwrap())
                                    .collect::<Vec<u8>>()
                                    .try_into()
                                    .unwrap();
                                u64::from_be_bytes(hash_bytes)
                            } else {
                                0
                            };

                            if let Err(err) = image_sender
                                .send((jpeg_data, pts, image_same, hash_value, hamming))
                                .await
                            {
                                log::error!("Failed to send image data through channel: {}", err);
                                break;
                            }

                            // Clear the filmstrip images for the next set
                            if frame_increment > 1 {
                                let increment = if frame_increment as usize > filmstrip_length {
                                    filmstrip_length
                                } else {
                                    frame_increment.into()
                                };
                                // remove the number of frames by frame_increment
                                if filmstrip_images.len() > increment as usize {
                                    filmstrip_images.drain(0..increment as usize);
                                } else {
                                    filmstrip_images.clear();
                                }
                            } else {
                                filmstrip_images.clear();
                            }
                        }
                    } else {
                        log::error!(
                            "Received image data with unexpected length: {}",
                            scaled_data.len()
                        );
                    }
                }
            }

            // Sleep for a short time to prevent a busy loop
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });
}

// constant for PAT PID
pub const PAT_PID: u16 = 0;
pub const TS_PACKET_SIZE: usize = 188;

#[derive(Debug)]
pub struct PatEntry {
    pub program_number: u16,
    pub pmt_pid: u16,
}

pub struct PmtEntry {
    pub stream_pid: u16,
    pub stream_type: u8, // Stream type (e.g., 0x02 for MPEG video)
    pub program_number: u16,
    pub descriptors: Vec<Descriptor>,
}

pub struct Pmt {
    pub entries: Vec<PmtEntry>,
    pub descriptors: Vec<Descriptor>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Descriptor {
    pub tag: u8,
    pub data: Vec<u8>,
}

#[derive(Clone, PartialEq)]
pub enum Codec {
    NONE,
    MPEG2,
    H264,
    H265,
}

impl fmt::Display for Codec {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Codec::NONE => write!(f, "NONE"),
            Codec::MPEG2 => write!(f, "MPEG2"),
            Codec::H264 => write!(f, "H264"),
            Codec::H265 => write!(f, "H265"),
        }
    }
}

// StreamData struct
#[derive(Serialize, Deserialize, Debug)]
pub struct StreamData {
    pub pid: u16,
    pub pmt_pid: u16,
    pub program_number: u16,
    pub stream_type: String, // Official stream type name in human readable form
    pub continuity_counter: u8,
    pub timestamp: u64,
    pub bitrate: u32,
    pub bitrate_max: u32,
    pub bitrate_min: u32,
    pub bitrate_avg: u32,
    pub iat: u64,
    pub iat_max: u64,
    pub iat_min: u64,
    pub iat_avg: u64,
    pub error_count: u32,
    pub current_error_count: u32,
    pub last_arrival_time: u64,
    pub last_sample_time: u64,
    pub start_time: u64,        // field for start time
    pub total_bits: u64,        // field for total bits
    pub total_bits_sample: u64, // field for total bits
    pub count: u32,             // field for count
    #[serde(skip)]
    pub packet: Arc<Vec<u8>>, // The actual MPEG-TS packet data
    pub packet_start: usize,    // Offset into the data
    pub packet_len: usize,      // Offset into the data
    // SMPTE 2110 fields
    pub rtp_timestamp: u32,
    pub rtp_payload_type: u8,
    pub rtp_payload_type_name: String,
    pub rtp_line_number: u16,
    pub rtp_line_offset: u16,
    pub rtp_line_length: u16,
    pub rtp_field_id: u8,
    pub rtp_line_continuation: u8,
    pub rtp_extended_sequence_number: u16,
    pub stream_type_number: u8,
    pub capture_time: u64,
    pub capture_iat: u64,
    pub source_ip: String,
    pub source_port: i32,
    pub has_image: u8,
    pub image_pts: u64,
    pub capture_iat_max: u64,
    pub log_message: String,
    pub probe_id: String,
    pub captions: String,
    pub pid_map: String,
    pub scte35: String,
    pub audio_loudness: String,
    pub pts: u64,
    pub pcr: u64,
}

impl Clone for StreamData {
    fn clone(&self) -> Self {
        StreamData {
            pid: self.pid,
            pmt_pid: self.pmt_pid,
            program_number: self.program_number,
            stream_type: self.stream_type.to_owned(),
            continuity_counter: self.continuity_counter,
            timestamp: self.timestamp,
            bitrate: self.bitrate,
            bitrate_max: self.bitrate_max,
            bitrate_min: self.bitrate_min,
            bitrate_avg: self.bitrate_avg,
            iat: self.iat,
            iat_max: self.iat_max,
            iat_min: self.iat_min,
            iat_avg: self.iat_avg,
            error_count: self.error_count,
            current_error_count: self.current_error_count,
            last_arrival_time: self.last_arrival_time,
            last_sample_time: self.last_sample_time,
            start_time: self.start_time,
            total_bits: self.total_bits,
            total_bits_sample: self.total_bits_sample,
            count: self.count,
            packet: Arc::clone(&self.packet),
            packet_start: self.packet_start,
            packet_len: self.packet_len,
            rtp_timestamp: self.rtp_timestamp,
            rtp_payload_type: self.rtp_payload_type,
            rtp_payload_type_name: self.rtp_payload_type_name.to_owned(),
            rtp_line_number: self.rtp_line_number,
            rtp_line_offset: self.rtp_line_offset,
            rtp_line_length: self.rtp_line_length,
            rtp_field_id: self.rtp_field_id,
            rtp_line_continuation: self.rtp_line_continuation,
            rtp_extended_sequence_number: self.rtp_extended_sequence_number,
            stream_type_number: self.stream_type_number,
            capture_time: self.capture_time,
            capture_iat: self.capture_iat,
            source_ip: self.source_ip.to_owned(),
            source_port: self.source_port,
            has_image: self.has_image,
            image_pts: self.image_pts,
            capture_iat_max: self.capture_iat_max,
            log_message: self.log_message.to_owned(),
            probe_id: self.probe_id.to_owned(),
            captions: self.captions.to_owned(),
            pid_map: self.pid_map.to_owned(),
            scte35: self.scte35.to_owned(),
            audio_loudness: self.audio_loudness.to_owned(),
            pts: self.pts,
            pcr: self.pcr,
        }
    }
}

// StreamData implementation
impl StreamData {
    pub fn new(
        packet: Arc<Vec<u8>>,
        packet_start: usize,
        packet_len: usize,
        pid: u16,
        stream_type: String,
        stream_type_number: u8,
        program_number: u16,
        start_time: u64,
        timestamp: u64,
        continuity_counter: u8,
        capture_timestamp: u64,
        capture_iat: u64,
        source_ip: String,
        source_port: i32,
        probe_id: String,
    ) -> Self {
        // convert capture_timestamp to unix timestamp in milliseconds since epoch
        let last_arrival_time = capture_timestamp;

        StreamData {
            pid,
            pmt_pid: 0xFFFF,
            program_number,
            stream_type,
            continuity_counter,
            timestamp,
            bitrate: 0,
            bitrate_max: 0,
            bitrate_min: 0,
            bitrate_avg: 0,
            iat: 0,
            iat_max: 0,
            iat_min: 0,
            iat_avg: 0,
            error_count: 0,
            current_error_count: 0,
            last_arrival_time,
            last_sample_time: start_time,
            start_time,           // Initialize start time
            total_bits: 0,        // Initialize total bits
            total_bits_sample: 0, // Initialize total bits
            count: 0,             // Initialize count
            packet,
            packet_start,
            packet_len,
            // SMPTE 2110 fields
            rtp_timestamp: 0,
            rtp_payload_type: 0,
            rtp_payload_type_name: "".to_string(),
            rtp_line_number: 0,
            rtp_line_offset: 0,
            rtp_line_length: 0,
            rtp_field_id: 0,
            rtp_line_continuation: 0,
            rtp_extended_sequence_number: 0,
            stream_type_number,
            capture_time: capture_timestamp,
            capture_iat,
            source_ip,
            source_port,
            // Initialize system stats fields from the SystemStats instance
            has_image: 0,
            image_pts: 0,
            capture_iat_max: capture_iat,
            log_message: "".to_string(),
            probe_id,
            captions: "".to_string(),
            pid_map: "".to_string(),
            scte35: "".to_string(),
            audio_loudness: "".to_string(),
            pts: 0,
            pcr: 0,
        }
    }
    // set RTP fields
    pub fn set_rtp_fields(
        &mut self,
        rtp_timestamp: u32,
        rtp_payload_type: u8,
        rtp_payload_type_name: String,
        rtp_line_number: u16,
        rtp_line_offset: u16,
        rtp_line_length: u16,
        rtp_field_id: u8,
        rtp_line_continuation: u8,
        rtp_extended_sequence_number: u16,
    ) {
        self.rtp_timestamp = rtp_timestamp;
        self.rtp_payload_type = rtp_payload_type;
        self.rtp_payload_type_name = rtp_payload_type_name;
        self.rtp_line_number = rtp_line_number;
        self.rtp_line_offset = rtp_line_offset;
        self.rtp_line_length = rtp_line_length;
        self.rtp_field_id = rtp_field_id;
        self.rtp_line_continuation = rtp_line_continuation;
        self.rtp_extended_sequence_number = rtp_extended_sequence_number;
    }
    pub fn update_stream_type(&mut self, stream_type: String) {
        self.stream_type = stream_type;
    }
    pub fn update_stream_type_number(&mut self, stream_type_number: u8) {
        self.stream_type_number = stream_type_number;
    }
    pub fn increment_error_count(&mut self, error_count: u32) {
        self.error_count += error_count;
        self.current_error_count = error_count;
    }
    pub fn increment_count(&mut self, count: u32) {
        self.count += count;
    }
    pub fn update_capture_time(&mut self, capture_timestamp: u64) {
        self.capture_time = capture_timestamp;
    }
    pub fn update_capture_iat(&mut self, capture_iat: u64) {
        self.capture_iat = capture_iat;

        // Store the IAT capture value in the global rolling window
        let mut iat_capture_window = IAT_CAPTURE_WINDOW.lock().unwrap();
        iat_capture_window.push_back(capture_iat);
        if iat_capture_window.len() > IAT_CAPTURE_WINDOW_SIZE {
            iat_capture_window.pop_front();
        }

        // Calculate the peak IAT within the global rolling window
        let peak_iat = *iat_capture_window.iter().max().unwrap_or(&0);
        *IAT_CAPTURE_PEAK.lock().unwrap() = peak_iat;

        // Update the capture_iat_max field based on the peak IAT value
        if peak_iat > self.capture_iat_max {
            self.capture_iat_max = peak_iat;
        } else if peak_iat < self.capture_iat_max {
            // If the peak IAT value is less than the current capture_iat_max,
            // update capture_iat_max to the new peak value
            self.capture_iat_max = peak_iat;
        }

        // If the global rolling window is full and the oldest value is equal to capture_iat_max,
        // recalculate capture_iat_max by finding the new maximum value in the window
        if iat_capture_window.len() == IAT_CAPTURE_WINDOW_SIZE
            && *iat_capture_window.front().unwrap() == self.capture_iat_max
        {
            self.capture_iat_max = *iat_capture_window.iter().max().unwrap_or(&0);
        }
    }
    pub fn update_source_ip(&mut self, source_ip: String) {
        self.source_ip = source_ip;
    }
    pub fn update_source_port(&mut self, source_port: u32) {
        self.source_port = source_port as i32;
    }
    pub fn update_program_number(&mut self, program_number: u16) {
        self.program_number = program_number;
    }
    pub fn set_continuity_counter(&mut self, continuity_counter: u8) {
        // check for continuity continuous increment and wrap around from 0 to 15
        let previous_continuity_counter = self.continuity_counter;
        self.continuity_counter = continuity_counter & 0x0F;

        // check if we incremented without loss
        if self.continuity_counter != (previous_continuity_counter + 1) & 0x0F
            && self.continuity_counter != previous_continuity_counter
        {
            // increment the error count by the difference between the current and previous continuity counter
            let error_count = if self.continuity_counter < previous_continuity_counter {
                (self.continuity_counter + 16) - previous_continuity_counter
            } else {
                self.continuity_counter - previous_continuity_counter
            } as u32;

            self.error_count += 1;
            self.current_error_count = error_count;

            error!(
                "Continuity Counter Error: PID: {} Previous: {} Current: {} Loss: {} Total Loss: {}",
                self.pid, previous_continuity_counter, self.continuity_counter, error_count, self.error_count
            );
        } else {
            // reset the error count
            self.current_error_count = 0;
        }

        self.continuity_counter = continuity_counter;
    }
    pub fn update_stats(&mut self, packet_size: usize) {
        let bits = packet_size as u64 * 8; // Convert packet size from bytes to bits
        self.total_bits += bits;
        self.total_bits_sample += bits;

        debug!(
            "{} Updating stats with packet size: {} and capture time: {} capture iat: {}",
            self.count, packet_size, self.capture_time, self.capture_iat
        );

        // Calculate elapsed time since the start of streaming and since the last sample
        let run_time_ms = self
            .capture_time
            .checked_sub(self.start_time)
            .unwrap_or_default();
        let elapsed_time_since_last_sample = self
            .capture_time
            .checked_sub(self.last_sample_time)
            .unwrap_or_default();

        // Only start calculating bitrate and IAT after a certain run time, e.g., 1000 ms
        if run_time_ms >= 1000 {
            if elapsed_time_since_last_sample >= 1000 {
                // Calculate the bitrate for the past second
                let bitrate = if self.total_bits_sample > 0 {
                    (self.total_bits_sample * 1000) / elapsed_time_since_last_sample
                } else {
                    0
                };

                // Update the moving average for the bitrate
                if self.count > 0 {
                    self.bitrate_avg = ((self.total_bits * 1000) / run_time_ms) as u32;
                } else {
                    self.bitrate_avg = bitrate as u32;
                }

                // Reset counters for the next interval
                self.total_bits_sample = 0;
                self.last_sample_time = self.capture_time;
                self.bitrate = bitrate as u32;
            }

            // Update max and min bitrates
            if self.bitrate > self.bitrate_max {
                self.bitrate_max = self.bitrate;
            }
            if self.bitrate < self.bitrate_min || self.bitrate_min == 0 {
                self.bitrate_min = self.bitrate;
            }

            // Calculate and update Inter-Arrival Time (IAT) and its statistics
            if self.count > 1 {
                let iat = self
                    .capture_time
                    .checked_sub(self.last_arrival_time)
                    .unwrap_or_default();
                self.iat = iat;

                // Update IAT max with proper initialization handling
                if run_time_ms >= 60000 && self.count > 100 {
                    if iat > self.iat_max {
                        self.iat_max = iat;
                    }

                    // Adjustments specific to IAT max startup handling
                    if iat < self.iat_min {
                        self.iat_min = iat;
                    }
                } else {
                    if (self.iat_min > iat) || (self.count == 1) {
                        self.iat_min = iat;
                    }
                    self.iat_max = 0;
                }

                self.iat_avg = (((self.iat_avg as u64 * self.count as u64) + iat)
                    / (self.count as u64 + 1)) as u64;
            }
            self.last_arrival_time = self.capture_time; // Update for the next packet's IAT calculation

            // Properly increment the count after all checks
            self.count += 1;
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Tr101290Errors {
    // p1 errors
    pub ts_sync_byte_errors: u32,
    pub sync_byte_errors: u32,
    pub continuity_counter_errors: u32,
    pub pat_errors: u32,
    pub pmt_errors: u32,
    pub pid_map_errors: u32,
    // p2 errors
    pub transport_error_indicator_errors: u32,
    pub crc_errors: u32,
    pub pcr_repetition_errors: u32,
    pub pcr_discontinuity_indicator_errors: u32,
    pub pcr_accuracy_errors: u32,
    pub pts_errors: u32,
    pub cat_errors: u32,
}

impl fmt::Display for Tr101290Errors {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "\
            TS Sync Byte Errors: {}, \
            Sync Byte Errors: {}, \
            Continuity Counter Errors: {}, \
            PAT Errors: {}, \
            PMT Errors: {}, \
            PID Map Errors: {}, \
            Transport Error Indicator Errors: {}, \
            CRC Errors: {}, \
            PCR Repetition Errors: {}, \
            PCR Discontinuity Indicator Errors: {}, \
            PCR Accuracy Errors: {}, \
            PTS Errors: {}, \
            CAT Errors: {}",
            self.ts_sync_byte_errors,
            self.sync_byte_errors,
            self.continuity_counter_errors,
            self.pat_errors,
            self.pmt_errors,
            self.pid_map_errors,
            // p2 errors
            self.transport_error_indicator_errors,
            self.crc_errors,
            self.pcr_repetition_errors,
            self.pcr_discontinuity_indicator_errors,
            self.pcr_accuracy_errors,
            self.pts_errors,
            self.cat_errors
        )
    }
}

impl Tr101290Errors {
    pub fn new() -> Self {
        Tr101290Errors {
            ts_sync_byte_errors: 0,
            sync_byte_errors: 0,
            continuity_counter_errors: 0,
            pat_errors: 0,
            pmt_errors: 0,
            pid_map_errors: 0,
            // p2
            transport_error_indicator_errors: 0,
            crc_errors: 0,
            pcr_repetition_errors: 0,
            pcr_discontinuity_indicator_errors: 0,
            pcr_accuracy_errors: 0,
            pts_errors: 0,
            cat_errors: 0,
        }
    }
}

// TR 101 290 Priority 1 Check
pub fn tr101290_p1_check(
    packet: &[u8],
    errors: &mut Tr101290Errors,
    pid: u16,
    stream_data: &mut StreamData,
) {
    // TS sync byte error
    if packet[0] != 0x47 {
        errors.ts_sync_byte_errors += 1;
    }

    // Sync byte error
    if packet[0] != 0x47 {
        errors.sync_byte_errors += 1;
    }

    // Continuity counter error
    if pid != 0x1FFF {
        errors.continuity_counter_errors = stream_data.error_count;
    }

    // PAT error
    if pid == 0 {
        let section_syntax_indicator = (packet[1] & 0x80) != 0;
        let section_length = (((packet[1] & 0x0F) as u16) << 8) | packet[2] as u16;
        if section_syntax_indicator && section_length >= 13 && packet.len() >= 8 {
            let table_id = packet[5];
            let section_number = packet[6];
            let last_section_number = packet[7];
            if table_id == 0x00 && section_number == 0x00 && last_section_number == 0x00 {
                let program_count = (section_length - 9) / 4;
                let mut i = 8;
                let mut valid_programs = 0;
                while i < packet.len() - 4 && valid_programs < program_count {
                    let program_number = (packet[i] as u16) << 8 | packet[i + 1] as u16;
                    if program_number != 0 {
                        valid_programs += 1;
                    }
                    i += 4;
                }
                if valid_programs == 0 {
                    errors.pat_errors += 1;
                }
            } else {
                errors.pat_errors += 1;
            }
        } else if packet.len() >= 8 && packet[5] == 0x00 {
            // Ignore PAT packets with section length less than 13
        } else {
            errors.pat_errors += 1;
        }
    }

    // PMT error
    if pid == stream_data.pmt_pid
        && stream_data.pmt_pid != 0
        && stream_data.pmt_pid != 0x1FFF
        && stream_data.pmt_pid != 0xFFFF
    {
        let section_syntax_indicator = (packet[1] & 0x80) != 0;
        let section_length = (((packet[1] & 0x0F) as u16) << 8) | packet[2] as u16;
        if section_syntax_indicator && packet.len() >= 4 && section_length >= 13 {
            let table_id = packet[3];
            if table_id == 0x02 {
                let program_number = (packet[4] as u16) << 8 | packet[5] as u16;
                let _version_number = (packet[6] & 0x3E) >> 1;
                let current_next_indicator = (packet[6] & 0x01) != 0;
                let section_number = packet[7];
                let last_section_number = packet[8];
                let program_info_length = (((packet[10] & 0x0F) as u16) << 8) | packet[11] as u16;

                if program_number == stream_data.program_number
                    && current_next_indicator
                    && section_number == 0
                    && last_section_number == 0
                    && program_info_length + 12 <= section_length
                {
                    // PMT is valid, don't increment the error counter
                } else {
                    errors.pmt_errors += 1;
                }
            } else {
                errors.pmt_errors += 1;
            }
        } else {
            errors.pmt_errors += 1;
        }
    }

    // PID map error
    if pid > 0
        && pid < 0x1FFF
        && pid != stream_data.pmt_pid
        && !PID_MAP.read().unwrap().contains_key(&pid)
    {
        errors.pid_map_errors += 1;
    }
}

// TR 101 290 Priority 2 Check
pub fn tr101290_p2_check(packet: &[u8], errors: &mut Tr101290Errors, stream_data: &mut StreamData) {
    // Transport error indicator
    if (packet[1] & 0x80) != 0 {
        errors.transport_error_indicator_errors += 1;
    }

    // CRC error
    let has_adaptation_field = (packet[3] & 0x20) != 0;
    let has_payload = (packet[3] & 0x10) != 0;
    if has_adaptation_field && has_payload {
        let adaptation_field_length = packet[4] as usize;
        let payload_start = 5 + adaptation_field_length;
        if payload_start + 4 <= packet.len() {
            let crc32_valid = (packet[1] & 0x80) != 0;
            if crc32_valid {
                let crc32 = u32::from_be_bytes([
                    packet[payload_start - 4],
                    packet[payload_start - 3],
                    packet[payload_start - 2],
                    packet[payload_start - 1],
                ]);
                let crc32_calculator = Crc::<u32>::new(&CRC_32_ISO_HDLC);
                let calculated_crc32 =
                    crc32_calculator.checksum(&packet[payload_start..packet.len() - 4]);
                if crc32 != calculated_crc32 {
                    errors.crc_errors += 1;
                }
            }
        }
    }

    // Check if the packet has an adaptation field
    let adaptation_field_control = (packet[3] & 0x30) >> 4;
    let has_adaptation_field = adaptation_field_control == 0b11 || adaptation_field_control == 0b10;

    let mut has_pcr = false;
    if has_adaptation_field && packet.len() >= 6 {
        let adaptation_field_length = packet[4] as usize;
        if packet.len() >= 6 + adaptation_field_length {
            // Check if the PCR flag is set in the adaptation field
            has_pcr = (packet[5] & 0x10) != 0;
        }
    }

    // PCR accuracy error
    if has_pcr {
        let pcr_base = u64::from_be_bytes([
            0,
            0,
            packet[6],
            packet[7],
            packet[8],
            packet[9],
            packet[10] & 0xFE,
            0,
        ]);
        let pcr_ext = (((packet[10] & 0x01) as u64) << 8) | (packet[11] as u64);
        let pcr = pcr_base * 300 + pcr_ext;
        if pcr != 0 {
            let max_pcr_accuracy_error = 500; // 500 nanoseconds
            let prev_pcr = stream_data.pcr;
            if (pcr as i64 - prev_pcr as i64).abs() > max_pcr_accuracy_error {
                errors.pcr_accuracy_errors += 1;
            }
            stream_data.pcr = pcr;
        }
    }

    // PCR repetition error
    if has_pcr {
        let pcr_base = u64::from_be_bytes([
            0,
            0,
            packet[6],
            packet[7],
            packet[8],
            packet[9],
            packet[10] & 0xFE,
            0,
        ]);
        let pcr_ext = (((packet[10] & 0x01) as u64) << 8) | (packet[11] as u64);
        let pcr = pcr_base * 300 + pcr_ext;
        if pcr != 0 {
            let max_pcr_repetition_interval = 40_000; // 40 ms in PCR units (27 MHz)
            let prev_pcr = stream_data.pcr;
            if pcr - prev_pcr > max_pcr_repetition_interval {
                errors.pcr_repetition_errors += 1;
            }
            stream_data.pcr = pcr;
        }
    }

    // PCR discontinuity indicator error
    let has_discontinuity_indicator = (packet[5] & 0x80) != 0;
    if has_discontinuity_indicator {
        let prev_discontinuity_state = stream_data.pcr & 0x01;
        if prev_discontinuity_state == 1 {
            errors.pcr_discontinuity_indicator_errors += 1;
        }
    }

    // PTS error
    let has_pts = (packet[7] & 0x80) != 0;
    if has_pts {
        let pts_high = u64::from_be_bytes([
            0,
            0,
            0,
            packet[9] & 0x0E,
            packet[10],
            packet[11],
            packet[12],
            packet[13] & 0xFE,
        ]);
        let pts_low = (((packet[13] & 0x01) as u64) << 8) | (packet[14] as u64);
        let pts = pts_high | pts_low;

        if pts != 0 {
            let max_pts_interval = 700_000; // 700 ms in PTS units (90 kHz)
            let prev_pts = stream_data.pts;

            if pts < prev_pts {
                // PTS is not monotonically increasing
                errors.pts_errors += 1;
            } else if pts - prev_pts > max_pts_interval {
                // PTS discontinuity exceeds the maximum allowed interval
                let max_pts_gap = 90_000; // 1 second in PTS units (90 kHz)
                if pts - prev_pts > max_pts_gap {
                    errors.pts_errors += 1;
                }
            }

            stream_data.pts = pts;
        }
    }

    // CAT error
    let has_cat = packet[3] == 0x01;
    if has_cat {
        let section_syntax_indicator = (packet[1] & 0x80) != 0;
        let section_length = (((packet[1] & 0x0F) as u16) << 8) | (packet[2] as u16);
        if !section_syntax_indicator || section_length < 9 {
            errors.cat_errors += 1;
        }
    }
}

// Implement a function to extract PID from a packet
pub fn extract_pid(packet: &[u8]) -> u16 {
    if packet.len() < TS_PACKET_SIZE {
        return 0; // Packet size is incorrect
    }

    let transport_error = (packet[1] & 0x80) != 0;
    if transport_error {
        return 0xFFFF; // Packet has a transport error
    }

    // Extract PID from packet
    ((packet[1] as u16 & 0x1F) << 8) | packet[2] as u16
}

// Story the PAT packet and PMT PID
pub struct PmtInfo {
    pub pid: u16,
    pub packet: Vec<u8>,
    pub program_number: u16,
}

// Helper function to parse PAT and update global PAT packet storage
pub fn parse_and_store_pat(packet: &[u8]) -> PmtInfo {
    let pat_entries = parse_pat(packet);
    let mut pmt_info = PmtInfo {
        pid: 0xFFFF,
        packet: Vec::new(),
        program_number: 0,
    };
    pmt_info.packet = packet.to_vec();

    debug!("ParseAndStorePAT: Found {} PAT entries {:?}", pat_entries.len(), pat_entries);

    let mut found_pmt = false;

    // look for the PMT Pid and Program Number that are greater zero and less than 0x1FFF for PMT PID
    for entry in pat_entries {
        if entry.pmt_pid > 0 && entry.pmt_pid <= 0x1FFF {
            if entry.pmt_pid > 0 && entry.pmt_pid <= 0x1FFF && entry.program_number > 0 {
                // TODO: return an array of all valid PMT PIDs and Program Numbers
                pmt_info.pid = entry.pmt_pid;
                pmt_info.program_number = entry.program_number;

                debug!(
                    "ParseAndStorePAT: Found Program Number: {} PMT PID: {}",
                    entry.program_number, entry.pmt_pid
                );
                found_pmt = true;
            } else {
                log::warn!(
                    "ParseAndStorePAT: PMT Pid OUT OF RANGE, Skipping Program Number: {} PMT PID: {}",
                    entry.program_number, entry.pmt_pid
                );
            }
        }
    }
    // print info on if we found a valid PMT PID or not
    if found_pmt {
        debug!("ParseAndStorePAT: Found PMT PID: {} Program Number: {}", pmt_info.pid, pmt_info.program_number);
    }
    pmt_info
}

// Helper function to extract descriptors with bounds checking
fn parse_descriptors(packet: &[u8], offset: usize, length: usize) -> Vec<Descriptor> {
    let mut descriptors = Vec::new();
    let mut i = offset;

    // Ensure that the descriptor parsing does not exceed packet bounds
    while i < offset + length && i + 2 <= packet.len() {
        let tag = packet[i];
        let len = packet[i + 1] as usize;

        // Check if the remaining data is sufficient to read the descriptor
        if i + 2 + len <= packet.len() {
            let data = packet[i + 2..i + 2 + len].to_vec();
            descriptors.push(Descriptor { tag, data });
        } else {
            // If there's not enough data for the descriptor, break the loop
            break;
        }

        i += 2 + len;
    }

    descriptors
}

// Parse PAT packets
pub fn parse_pat(packet: &[u8]) -> Vec<PatEntry> {
    let mut entries = Vec::new();

    if packet.len() < TS_PACKET_SIZE {
        return entries;
    }

    let pusi = (packet[1] & 0x40) != 0;
    if !pusi {
        return entries;
    }

    let adaptation_field_control = (packet[3] & 0x30) >> 4;
    let mut offset = 4;

    if adaptation_field_control == 0x02 || adaptation_field_control == 0x03 {
        let adaptation_field_length = packet[4] as usize;
        offset += 1 + adaptation_field_length;
    }

    let pointer_field = packet[offset] as usize;
    offset += 1 + pointer_field;

    while offset + 4 <= packet.len() {
        let program_number = ((packet[offset] as u16) << 8) | (packet[offset + 1] as u16);
        let pmt_pid = (((packet[offset + 2] as u16) & 0x1F) << 8) | (packet[offset + 3] as u16);

        if program_number > 0 && pmt_pid > 0 && pmt_pid <= 0x1FFF && program_number < 5000 {
            entries.push(PatEntry {
                program_number,
                pmt_pid,
            });
            debug!(
                "ParsePAT: Found Program Number: {} PMT PID: {}",
                program_number, pmt_pid
            );
        } else {
            if program_number <= 0 {
                debug!(
                    "ParsePAT: Skipping Program Number <= 0: {} PMT PID: {}",
                    program_number, pmt_pid
                );
            } else if pmt_pid <= 0 {
                debug!(
                    "ParsePAT: Skipping PMT PID <= 0: {} Program Number: {}",
                    pmt_pid, program_number
                );
            } else if pmt_pid > 0x1FFF {
                debug!(
                    "ParsePAT: Skipping PMT PID >= 0x1FFF: {} Program Number: {}",
                    pmt_pid, program_number
                );
            }
        }

        offset += 4;
    }

    entries
}

// Parse PMT packets
pub fn parse_pmt(packet: &[u8]) -> Pmt {
    let mut entries = Vec::new();

    let adaptation_field_control = (packet[3] & 0x30) >> 4;
    let mut offset = 0;

    if adaptation_field_control == 0x02 || adaptation_field_control == 0x03 {
        let adaptation_field_length = packet[4] as usize;
        offset += 1 + adaptation_field_length;
    }

    if offset + 4 > packet.len() {
        error!("ParsePMT: Packet size is incorrect: {}", packet.len());
        return Pmt {
            entries,
            descriptors: Vec::new(),
        };
    }

    let program_number = ((packet[8 + offset] as u16) << 8) | (packet[9 + offset] as u16);

    let section_length = (((packet[6+ offset] as usize) & 0x0F) << 8) | packet[7 + offset] as usize;
    let program_info_length = (((packet[15 + offset] as usize) & 0x0F) << 8) | packet[16 + offset] as usize;
    let mut i = 17 + program_info_length;

    // Parse program descriptors
    let descriptors = parse_descriptors(packet, 17 + offset, program_info_length);

    while i + 5 + offset <= packet.len() && i < 17 + section_length - 4 {
        let stream_type = packet[i + offset];
        let stream_pid = (((packet[i + 1 + offset] as u16) & 0x1F) << 8) | (packet[i + 2 + offset] as u16);
        let es_info_length = (((packet[i + 3 + offset] as usize) & 0x0F) << 8) | packet[i + 4 + offset] as usize;

        // Parse ES descriptors
        let es_descriptors = parse_descriptors(packet, i + 5 + offset, es_info_length);

        entries.push(PmtEntry {
            stream_pid,
            stream_type,
            program_number,
            descriptors: es_descriptors,
        });

        i += 5 + es_info_length;
    }

    Pmt { entries, descriptors }
}

// Invoke this function for each MPEG-TS packet
pub fn process_packet(
    stream_data_packet: &mut StreamData,
    errors: &mut Tr101290Errors,
    pmt_pid: u16,
    probe_id: String,
) {
    let packet: &[u8] = &stream_data_packet.packet[stream_data_packet.packet_start
        ..stream_data_packet.packet_start + stream_data_packet.packet_len];

    let mut updated_stream_data = stream_data_packet.clone();

    if stream_data_packet.pid != 0x1FFF {
        tr101290_p1_check(
            packet,
            errors,
            stream_data_packet.pid,
            &mut updated_stream_data,
        );
        tr101290_p2_check(packet, errors, &mut updated_stream_data);
    }

    // copy the pts and pcr values back to the original stream_data_packet
    stream_data_packet.pts = updated_stream_data.pts;
    stream_data_packet.pcr = updated_stream_data.pcr;

    let pid = stream_data_packet.pid;

    let mut pid_map = PID_MAP.write().unwrap();

    // Check if the PID map already has an entry for this PID
    match pid_map.get_mut(&pid) {
        Some(stream_data_arc) => {
            // Existing StreamData instance found, update it
            let mut stream_data = Arc::clone(stream_data_arc);
            Arc::make_mut(&mut stream_data).update_capture_time(stream_data_packet.capture_time);
            Arc::make_mut(&mut stream_data)
                .update_stream_type_number(stream_data_packet.stream_type_number);
            Arc::make_mut(&mut stream_data).update_stats(packet.len());
            Arc::make_mut(&mut stream_data).update_capture_iat(stream_data_packet.capture_iat);
            if stream_data.pid != 0x1FFF {
                Arc::make_mut(&mut stream_data)
                    .set_continuity_counter(stream_data_packet.continuity_counter);
            }

            if stream_data_packet.timestamp != 0 {
                Arc::make_mut(&mut stream_data).timestamp = stream_data_packet.timestamp;
            }

            // update the pmt_pid from the stream_data_packet to stream_data
            Arc::make_mut(&mut stream_data).pmt_pid = pmt_pid;

            // update the program_number from the stream_data_packet to stream_data
            Arc::make_mut(&mut stream_data).program_number = stream_data_packet.program_number;

            // write the current pts and pcr values to the stream_data
            Arc::make_mut(&mut stream_data).pts = stream_data_packet.pts;
            Arc::make_mut(&mut stream_data).pcr = stream_data_packet.pcr;

            // print out each field of structure
            debug!("Modify PID: process_packet [{}] pid: {} stream_type: {} bitrate: {} bitrate_max: {} bitrate_min: {} bitrate_avg: {} iat: {} iat_max: {} iat_min: {} iat_avg: {} errors: {} continuity_counter: {} timestamp: {} uptime: {} packet_offset: {}, packet_len: {}",
                stream_data.program_number, stream_data.pid, stream_data.stream_type,
                stream_data.bitrate, stream_data.bitrate_max, stream_data.bitrate_min,
                stream_data.bitrate_avg, stream_data.iat, stream_data.iat_max, stream_data.iat_min,
                stream_data.iat_avg, stream_data.error_count, stream_data.continuity_counter,
                stream_data.timestamp, 0/*uptime*/, stream_data_packet.packet_start, stream_data_packet.packet_len);

            stream_data_packet.bitrate = stream_data.bitrate;
            stream_data_packet.bitrate_avg = stream_data.bitrate_avg;
            stream_data_packet.bitrate_max = stream_data.bitrate_max;
            stream_data_packet.bitrate_min = stream_data.bitrate_min;
            stream_data_packet.iat = stream_data.iat;
            stream_data_packet.iat_avg = stream_data.iat_avg;
            stream_data_packet.iat_max = stream_data.iat_max;
            stream_data_packet.iat_min = stream_data.iat_min;
            stream_data_packet.stream_type = stream_data.stream_type.clone();
            stream_data_packet.start_time = stream_data.start_time;
            stream_data_packet.last_arrival_time = stream_data.last_arrival_time;
            stream_data_packet.last_sample_time = stream_data.last_sample_time;
            stream_data_packet.total_bits_sample = stream_data.total_bits_sample;
            stream_data_packet.total_bits = stream_data.total_bits;
            stream_data_packet.count = stream_data.count;
            stream_data_packet.pmt_pid = pmt_pid;
            stream_data_packet.program_number = stream_data.program_number;
            stream_data_packet.error_count = stream_data.error_count;
            stream_data_packet.current_error_count = stream_data.current_error_count;
            if stream_data_packet.timestamp == 0 {
                stream_data_packet.timestamp = stream_data.timestamp;
            }
            stream_data_packet.probe_id = probe_id.clone();

            // write the stream_data back to the pid_map with modified values
            pid_map.insert(pid, stream_data);
        }
        None => {
            // No StreamData instance found for this PID, possibly no PMT yet
            if pmt_pid != 0xFFFF {
                debug!("ProcessPacket: New PID {} Found, adding to PID map.", pid);
            } else {
                // PMT packet not found yet, add the stream_data_packet to the pid_map
                // OS and Network stats
                let mut stream_data = Arc::new(StreamData::new(
                    Arc::new(Vec::new()), // Ensure packet_data is Arc<Vec<u8>>
                    0,
                    0,
                    stream_data_packet.pid,
                    stream_data_packet.stream_type.clone(),
                    stream_data_packet.stream_type_number.clone(),
                    stream_data_packet.program_number,
                    stream_data_packet.start_time,
                    stream_data_packet.timestamp,
                    stream_data_packet.continuity_counter,
                    stream_data_packet.capture_time,
                    stream_data_packet.capture_iat,
                    stream_data_packet.source_ip.clone(),
                    stream_data_packet.source_port,
                    probe_id.clone(),
                ));
                Arc::make_mut(&mut stream_data).update_stats(packet.len());
                Arc::make_mut(&mut stream_data).update_capture_iat(stream_data_packet.capture_iat);
                // update continuity counter
                if stream_data_packet.pid != 0x1FFF {
                    Arc::make_mut(&mut stream_data)
                        .set_continuity_counter(stream_data_packet.continuity_counter);
                }

                info!(
                    "[{}] Adding PID: {} [{}][{}][{}] StreamType: [{}] {}",
                    stream_data.program_number,
                    stream_data.pid,
                    stream_data.capture_time,
                    stream_data.capture_iat,
                    stream_data.continuity_counter,
                    stream_data.stream_type_number,
                    stream_data.stream_type
                );

                pid_map.insert(pid, stream_data);
            }
        }
    }
}

pub fn cleanup_stale_streams(clear_stream_timeout: u64) {
    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis() as u64;

    let mut pid_map = PID_MAP.write().unwrap();
    let stale_pids: Vec<u16> = pid_map
        .iter()
        .filter_map(|(&pid, stream_data)| {
            if current_time.saturating_sub(stream_data.last_arrival_time) > clear_stream_timeout{
                Some(pid)
            } else {
                None
            }
        })
        .collect();

    for pid in stale_pids {
        if let Some(removed_stream) = pid_map.remove(&pid) {
            info!(
                "Removed stale stream: PID {}, Program {}, Last seen {} ms ago",
                pid,
                removed_stream.program_number,
                current_time.saturating_sub(removed_stream.last_arrival_time)
            );
        }
    }
}

// Use the stored PAT packet
pub fn update_pid_map(
    pmt_packet: &[u8],
    last_pat_packet: &[u8],
    capture_timestamp: u64,
    capture_iat: u64,
    source_ip: String,
    source_port: i32,
    probe_id: String,
) -> u16 {
    let mut pid_map = PID_MAP.write().unwrap();
    let mut program_number_result = 0;

    // Process the stored PAT packet to find program numbers and corresponding PMT PIDs
    let program_pids = last_pat_packet
        .chunks_exact(TS_PACKET_SIZE)
        .flat_map(parse_pat)
        .collect::<Vec<_>>();

    for pat_entry in program_pids.iter() {
        let program_number = pat_entry.program_number;
        let pmt_pid = pat_entry.pmt_pid;

        // Log for debugging
        debug!(
            "UpdatePIDmap: Processing Program Number: {}, PMT PID: {}",
            program_number, pmt_pid
        );
        program_number_result = program_number;

        // Ensure the current PMT packet matches the PMT PID from the PAT
        //if extract_pid(pmt_packet) == pmt_pid {
            let pmt = parse_pmt(pmt_packet);

            for pmt_entry in pmt.entries.iter() {
                debug!(
                    "UpdatePIDmap: Processing PMT PID: {} for Stream PID: {} Type {}",
                    pmt_pid, pmt_entry.stream_pid, pmt_entry.stream_type
                );

                let stream_pid = pmt_entry.stream_pid;
                let stream_type_num = pmt_entry.stream_type;
                let stream_type = match pmt_entry.stream_type {
                    0x00 => "Reserved",
                    0x01 => "ISO/IEC 11172 MPEG-1 Video",
                    0x02 => "ISO/IEC 13818-2 MPEG-2 Video",
                    0x03 => "ISO/IEC 11172 MPEG-1 Audio",
                    0x04 => "ISO/IEC 13818-3 MPEG-2 Audio",
                    0x05 => "ISO/IEC 13818-1 Private Section",
                    0x06 => "ISO/IEC 13818-1 Private PES data packets",
                    0x07 => "ISO/IEC 13522 MHEG",
                    0x08 => "ISO/IEC 13818-1 Annex A DSM CC",
                    0x09 => "H222.1",
                    0x0A => "ISO/IEC 13818-6 type A",
                    0x0B => "ISO/IEC 13818-6 type B",
                    0x0C => "ISO/IEC 13818-6 type C",
                    0x0D => "ISO/IEC 13818-6 type D",
                    0x0E => "ISO/IEC 13818-1 auxillary",
                    0x0F => "13818-7 AAC Audio with ADTS transport syntax",
                    0x10 => "14496-2 Visual (MPEG-4 part 2 video)",
                    0x11 => "14496-3 MPEG-4 Audio with LATM transport syntax (14496-3/AMD 1)",
                    0x12 => "14496-1 SL-packetized or FlexMux stream in PES packets",
                    0x13 => "14496-1 SL-packetized or FlexMux stream in 14496 sections",
                    0x14 => "ISO/IEC 13818-6 Synchronized Download Protocol",
                    0x15 => "Metadata in PES packets",
                    0x16 => "Metadata in metadata_sections",
                    0x17 => "Metadata in 13818-6 Data Carousel",
                    0x18 => "Metadata in 13818-6 Object Carousel",
                    0x19 => "Metadata in 13818-6 Synchronized Download Protocol",
                    0x1A => "13818-11 MPEG-2 IPMP stream",
                    0x1B => "H.264/14496-10 video (MPEG-4/AVC)",
                    0x24 => "H.265 video (MPEG-H/HEVC)",
                    0x42 => "AVS Video",
                    0x7F => "IPMP stream",
                    0x81 => "ATSC A/52 AC-3",
                    0x86 => "SCTE 35 Splice Information Table",
                    0x87 => "ATSC A/52e AC-3",
                    _ if pmt_entry.stream_type < 0x80 => "ISO/IEC 13818-1 reserved",
                    _ => "User Private",
                };

                let timestamp = current_unix_timestamp_ms().unwrap_or(0);

                if !pid_map.contains_key(&stream_pid) {
                    let mut stream_data = Arc::new(StreamData::new(
                        Arc::new(Vec::new()), // Ensure packet_data is Arc<Vec<u8>>
                        0,
                        0,
                        stream_pid,
                        stream_type.to_string(),
                        stream_type_num,
                        program_number,
                        timestamp,
                        timestamp,
                        0,
                        capture_timestamp,
                        capture_iat,
                        source_ip.clone(),
                        source_port,
                        probe_id.clone(),
                    ));
                    // update stream_data stats
                    Arc::make_mut(&mut stream_data).update_stats(pmt_packet.len());
                    Arc::make_mut(&mut stream_data).update_capture_iat(capture_iat);

                    // nicer shorter basic start of capture information, not stats or errors
                    info!("[{}] update_pid_map Create Stream: [{}] pid {} stream_type {}/{} continuity {}",
                        stream_data.capture_time, stream_data.program_number,
                        stream_data.pid, stream_data.stream_type, stream_data.stream_type_number,
                        stream_data.continuity_counter);

                    pid_map.insert(stream_pid, stream_data);
                } else {
                    // get the stream data so we can update it
                    let stream_data_arc = pid_map.get_mut(&stream_pid).unwrap();
                    let mut stream_data = Arc::clone(stream_data_arc);
                    let stream_len = stream_data.packet_len;

                    // update the stream type
                    Arc::make_mut(&mut stream_data).update_stream_type(stream_type.to_string());
                    Arc::make_mut(&mut stream_data)
                        .update_stream_type_number(pmt_entry.stream_type);
                    Arc::make_mut(&mut stream_data).update_stats(stream_len);
                    Arc::make_mut(&mut stream_data).update_capture_iat(capture_iat);

                    debug!("[{}] update_pid_map Update Stream: [{}] pid {} stream_type {}/{} continuity {}",
                        stream_data.capture_time, stream_data.program_number,
                        stream_data.pid, stream_data.stream_type, stream_data.stream_type_number,
                        stream_data.continuity_counter);

                    // write the stream_data back to the pid_map with modified values
                    pid_map.insert(stream_pid, stream_data);
                }
            }
        //} else {
        //    error!("UpdatePIDmap: Skipping PMT PID: {} as it does not match with current PMT packet PID", pmt_pid);
        //}
    }
    program_number_result
}

pub fn determine_stream_type(pid: u16) -> String {
    let pid_map = PID_MAP.read().unwrap();

    // check if pid already is mapped, if so return the stream type already stored
    if let Some(stream_data) = pid_map.get(&pid) {
        return stream_data.stream_type.clone();
    }

    pid_map
        .get(&pid)
        .map(|stream_data| stream_data.stream_type.clone())
        .unwrap_or_else(|| "unknown".to_string())
}

pub fn get_pid_map() -> AHashMap<u16, Arc<StreamData>> {
    let pid_map = PID_MAP.read().unwrap();
    // clone the pid_map so we can return it
    pid_map.clone()
}

pub fn determine_stream_type_number(pid: u16) -> u8 {
    let pid_map = PID_MAP.read().unwrap();

    // check if pid already is mapped, if so return the stream type already stored
    if let Some(stream_data) = pid_map.get(&pid) {
        return stream_data.stream_type_number.clone();
    }

    pid_map
        .get(&pid)
        .map(|stream_data| stream_data.stream_type_number.clone())
        .unwrap_or_else(|| 0)
}

pub fn determine_stream_program_number(pid: u16) -> u16 {
    let pid_map = PID_MAP.read().unwrap();

    // check if pid already is mapped, if so return the stream type already stored
    if let Some(stream_data) = pid_map.get(&pid) {
        return stream_data.program_number.clone();
    }

    pid_map
        .get(&pid)
        .map(|stream_data| stream_data.program_number.clone())
        .unwrap_or_else(|| 0)
}

// Helper function to identify the video PID from the stored PAT packet and return the PID and codec
pub fn identify_video_pid(pmt_packet: &[u8]) -> Option<(u16, Codec)> {
    let pmt = parse_pmt(pmt_packet);
    pmt.entries.iter().find_map(|entry| {
        let codec = match entry.stream_type {
            0x01..=0x02 => Some(Codec::MPEG2), // MPEG-2 Video
            0x1B => Some(Codec::H264),         // H.264 Video
            0x24 => Some(Codec::H265),         // H.265 Video
            _ => None,
        };
        codec.map(|c| (entry.stream_pid, c))
    })
}

// Process the packet and return a vector of MPEG-TS packets
pub fn process_mpegts_packet(
    payload_offset: usize,
    packet: Arc<Vec<u8>>,
    packet_size: usize,
    start_time: u64,
    capture_timestamp: u64,
    capture_iat: u64,
    source_ip: String,
    source_port: i32,
    probe_id: String,
) -> Vec<StreamData> {
    let mut start = payload_offset;
    let mut read_size = packet_size;
    let mut streams = Vec::new();

    let len = packet.len();

    while start + read_size <= len {
        let chunk = &packet[start..start + read_size];
        if chunk[0] == 0x47 {
            // Check for MPEG-TS sync byte
            read_size = packet_size; // reset read_size

            let pid = extract_pid(chunk);

            let stream_type = determine_stream_type(pid);
            let stream_type_number = determine_stream_type_number(pid);
            let stream_program_number = determine_stream_program_number(pid);
            let timestamp = extract_timestamp(chunk);
            let continuity_counter = chunk[3] & 0x0F;

            let mut stream_data = StreamData::new(
                Arc::clone(&packet),
                start,
                packet_size,
                pid,
                stream_type,
                stream_type_number,
                stream_program_number,
                start_time,
                timestamp,
                continuity_counter,
                capture_timestamp,
                capture_iat,
                source_ip.clone(),
                source_port,
                probe_id.clone(),
            );
            stream_data.update_stats(packet_size);
            stream_data.update_capture_iat(capture_iat);
            streams.push(stream_data);
        } else {
            error!("ProcessPacket: Not MPEG-TS");
            read_size = 1; // Skip to the next byte
        }
        start += read_size;
    }

    streams
}

fn extract_timestamp(chunk: &[u8]) -> u64 {
    let adaptation_field_control = (chunk[3] & 0x30) >> 4;

    if adaptation_field_control == 0b10 || adaptation_field_control == 0b11 {
        // Adaptation field is present
        let adaptation_field_length = chunk[4] as usize;

        if adaptation_field_length > 0 && (chunk[5] & 0x10) != 0 {
            // PCR is present
            let pcr_base = ((chunk[6] as u64) << 25)
                | ((chunk[7] as u64) << 17)
                | ((chunk[8] as u64) << 9)
                | ((chunk[9] as u64) << 1)
                | ((chunk[10] as u64) >> 7);
            let pcr_ext = (((chunk[10] as u64) & 0x01) << 8) | (chunk[11] as u64);
            let pcr = pcr_base * 300 + pcr_ext;
            pcr
        } else {
            0 // Default value when PCR is not present
        }
    } else {
        0 // Default value when adaptation field is not present
    }
}
