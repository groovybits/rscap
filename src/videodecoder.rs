/*
 * This file contains the implementation of the VideoDecoder and VideoProcessor structs
 * The VideoDecoder struct is responsible for decoding the H264 data and extracting the SEI messages
 * TODO: Add support for decoding H264 and extracting SEI messages of all types
*/

use anyhow::Result as AnyResult;
use h264_reader::nal::UnitType;
use h264_reader::push::NalInterest;
use h264_reader::{
    annexb::AnnexBReader,
    nal::{pps, sps, Nal, RefNal},
    Context as H264Context,
};
use std::sync::{Arc, Mutex};

pub struct VideoDecoder {
    //frame_buffer: Vec<Vec<u8>>,
    h264_ctx: H264Context,
}

impl VideoDecoder {
    pub fn new() -> AnyResult<Self> {
        Ok(VideoDecoder {
            //frame_buffer: Vec::new(),
            h264_ctx: H264Context::new(),
        })
    }

    pub fn process_packet(&mut self, packet_data: &[u8]) -> AnyResult<()> {
        // Using the existing h264_ctx instead of creating a new Context
        let mut annexb_reader = AnnexBReader::accumulate(|nal: RefNal<'_>| {
            if !nal.is_complete() {
                return NalInterest::Buffer;
            }

            let hdr = match nal.header() {
                Ok(h) => h,
                Err(_) => return NalInterest::Buffer, // Proper error handling
            };

            match hdr.nal_unit_type() {
                UnitType::SeqParameterSet => {
                    if let Ok(sps) = sps::SeqParameterSet::from_bits(nal.rbsp_bits()) {
                        self.h264_ctx.put_seq_param_set(sps);
                    }
                }
                UnitType::PicParameterSet => {
                    if let Ok(pps) =
                        pps::PicParameterSet::from_bits(&self.h264_ctx, nal.rbsp_bits())
                    {
                        self.h264_ctx.put_pic_param_set(pps);
                    }
                }
                // ... handle other NAL unit types as necessary ...
                _ => {}
            }
            NalInterest::Buffer
        });

        annexb_reader.push(packet_data);
        annexb_reader.reset();

        // Additional processing using self.h264_ctx as needed
        // ...

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
}
