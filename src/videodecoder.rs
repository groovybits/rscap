/*
 * This file contains the implementation of the VideoDecoder and VideoProcessor structs
 * The VideoDecoder struct is responsible for decoding the H264 data and extracting the SEI messages
 * TODO: Add support for decoding H264 and extracting SEI messages of all types
*/

use anyhow::Result as AnyResult;
use h264_reader::nal::UnitType;
use h264_reader::push::NalFragmentHandler;
use h264_reader::push::NalInterest;
use h264_reader::{
    annexb::AnnexBReader,
    nal::{pps, sps, Nal, RefNal},
    Context as H264Context,
};
use log::info;
use std::sync::{Arc, Mutex};

struct H264NalFragmentHandler {
    h264_ctx: H264Context,
}

impl NalFragmentHandler for H264NalFragmentHandler {
    fn handle_nal(&mut self, nal: RefNal<'_>) -> NalInterest {
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
                    info!("SPS: {:?}", sps);
                    self.h264_ctx.put_seq_param_set(sps);
                }
            }
            UnitType::PicParameterSet => {
                if let Ok(pps) = pps::PicParameterSet::from_bits(&self.h264_ctx, nal.rbsp_bits()) {
                    info!("PPS: {:?}", pps);
                    self.h264_ctx.put_pic_param_set(pps);
                }
            }
            // ... handle other NAL unit types as necessary ...
            _ => {
                info!("NAL: {:?}", hdr.nal_unit_type());
            }
        }
        NalInterest::Buffer
    }
}

pub struct VideoDecoder {
    nal_handler: H264NalFragmentHandler,
    annexb_reader: AnnexBReader<'static, Box<dyn NalFragmentHandler + Send>>,
}

impl VideoDecoder {
    pub fn new() -> AnyResult<Self> {
        let h264_ctx = H264Context::new();
        let cloned_ctx = h264_ctx.clone(); // Clone the context for the closure

        let annexb_reader = AnnexBReader::accumulate(Box::new(move |nal: RefNal<'_>| {
            let mut handler = H264NalFragmentHandler {
                h264_ctx: cloned_ctx.clone(),
            };
            handler.handle_nal(nal)
        }));

        /*let mut annexb_reader = AnnexBReader::accumulate(|nal: RefNal<'_>| {
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
                        info!("SPS: {:?}", sps);
                        h264_ctx.put_seq_param_set(sps);
                    }
                }
                UnitType::PicParameterSet => {
                    if let Ok(pps) = pps::PicParameterSet::from_bits(&h264_ctx, nal.rbsp_bits()) {
                        info!("PPS: {:?}", pps);
                        h264_ctx.put_pic_param_set(pps);
                    }
                }
                // ... handle other NAL unit types as necessary ...
                _ => {
                    info!("NAL: {:?}", hdr.nal_unit_type());
                }
            }
            NalInterest::Buffer
        });
        */
        Ok(VideoDecoder {
            //frame_buffer: Vec::new(),
            nal_handler: nal_handler,
            annexb_reader: annexb_reader,
        })
    }

    pub fn process_packet(&mut self, packet_data: &[u8]) -> AnyResult<()> {
        // Using the existing h264_ctx instead of creating a new Context

        self.annexb_reader.push(packet_data);
        //self.annexb_reader.reset();

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
