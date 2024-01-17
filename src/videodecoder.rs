/*
 * This file contains the implementation of the VideoDecoder and VideoProcessor structs
 * The VideoDecoder struct is responsible for decoding the H264 data and extracting the SEI messages
 * TODO: Add support for decoding H264 and extracting SEI messages of all types
*/

use anyhow::{Context as AnyContext, Result as AnyResult};
use h264_reader::{
    avcc::AvcDecoderConfigurationRecord,
    nal::{self, sei::pic_timing::PicTiming, sei::SeiReader, Nal, NalHeader},
};
use log::debug;
use std::convert::TryFrom;
use std::error::Error;
use std::format;
use std::io::Cursor;
use std::sync::{Arc, Mutex}; // Using anyhow for error handling

// Custom Error type for AVC errors
#[derive(Debug)]
struct AvccErrorWrapper(String);

impl std::fmt::Display for AvccErrorWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "AVC Error: {}", self.0)
    }
}

impl Error for AvccErrorWrapper {}

impl From<h264_reader::avcc::AvccError> for AvccErrorWrapper {
    fn from(err: h264_reader::avcc::AvccError) -> Self {
        AvccErrorWrapper(format!("{:?}", err))
    }
}

pub struct VideoDecoder {
    frame_buffer: Vec<Vec<u8>>,
    avc_config_record: Option<Vec<u8>>,
}

impl VideoDecoder {
    pub fn new() -> AnyResult<Self> {
        Ok(VideoDecoder {
            frame_buffer: Vec::new(),
            avc_config_record: None,
        })
    }

    fn process_h264_data(&mut self, data: &[u8], ctx: &mut H264Context) -> AnyResult<()> {
        // Process the H264 data here
        // Assuming `nal::parse_nal_units` is the correct method to parse NAL units
        for nal in nal::parse_nal_units(data) {
            match nal.header() {
                NalHeader::Slice { .. } => {
                    // Process slice data here
                }
                NalHeader::Sei => {
                    // Process SEI message
                    if let Ok(sei) = SeiMessage::try_from(nal) {
                        for message in sei.messages() {
                            match message {
                                SeiMessage::PicTiming(pic_timing) => {
                                    // Extract the pic_timing data here
                                    println!("PicTiming: {:?}", pic_timing);
                                }
                                _ => {}
                            }
                        }
                    }
                }
                // Handle other NAL types as needed
                _ => {}
            }
        }
        Ok(())
    }

    pub fn process_packet(&mut self, packet_data: &[u8]) -> AnyResult<()> {
        if let Some(avc_config_record) = self.extract_avc_config_record(packet_data) {
            let record = AvcDecoderConfigurationRecord::try_from(avc_config_record.as_slice())
                .map_err(|e| anyhow::Error::new(e))
                .context("Failed to create AVC Decoder Configuration Record")?;

            let mut ctx = record
                .create_context()
                .map_err(|e| anyhow::Error::new(e))
                .context("Failed to create AVC Context")?;

            // Now process the H264 data in the frame buffer
            for frame in &self.frame_buffer {
                self.process_h264_data(frame, &mut ctx)
                    .context("Failed to process H264 data")?;
            }
        }
        Ok(())
    }

    // Assuming `extract_avc_config_record` is a function you implement to extract the AVC configuration record from MPEG-TS packets
    pub fn extract_avc_config_record(&self, _ts_packet: &[u8]) -> Option<Vec<u8>> {
        // Extract AVC configuration record from the TS packet
        // Placeholder for implementation
        // This needs to be implemented based on your specific MPEG-TS packet structure
        None
    }

    pub fn start(&self) -> Result<(), anyhow::Error> {
        Ok(())
    }

    pub fn stop(&self) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

pub struct VideoProcessor {
    pub decoder: Arc<Mutex<VideoDecoder>>,
}

impl VideoProcessor {
    pub fn initialize() -> AnyResult<Self> {
        let decoder = VideoDecoder::new()?;
        Ok(VideoProcessor {
            decoder: Arc::new(Mutex::new(decoder)),
        })
    }
    pub fn feed_packet(&self, packet_data: &[u8]) -> AnyResult<()> {
        let mut decoder = self.decoder.lock().unwrap();
        decoder.process_packet(packet_data)
    }

    pub fn start(&self) -> Result<(), anyhow::Error> {
        let decoder = self.decoder.lock().unwrap();
        decoder.start()
    }

    pub fn stop(&self) -> Result<(), anyhow::Error> {
        let decoder = self.decoder.lock().unwrap();
        decoder.stop()
    }
}
