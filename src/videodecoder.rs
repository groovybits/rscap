extern crate ffmpeg_next as ffmpeg;
use ffmpeg::{codec, decoder, encoder, format, frame, sys as ffmpeg_sys, Packet};
use std::sync::{Arc, Mutex};

pub struct VideoDecoder {
    decoder: decoder::Video,
}

impl VideoDecoder {
    // Initialize with codec parameters
    pub fn new(codec_params: codec::Parameters) -> Result<Self, ffmpeg::Error> {
        ffmpeg::init()?;

        // Create a context from the codec parameters
        let mut codec_context = codec::context::Context::from_parameters(codec_params)?;
        let decoder = codec_context.decoder().video()?;
        decoder.open()?;

        Ok(VideoDecoder { decoder })
    }

    // Process a video packet
    pub fn process_packet(&mut self, packet_data: &[u8]) -> Result<(), ffmpeg::Error> {
        let mut packet = Packet::empty();
        if let Some(data) = packet.data_mut() {
            data.clone_from_slice(packet_data);
        }
        self.decoder.send_packet(&packet)?;

        let mut decoded = frame::Video::empty();
        while self.decoder.receive_frame(&mut decoded).is_ok() {
            save_frame_as_jpeg(&decoded, "frame.jpg")?;
        }

        Ok(())
    }
}

pub struct VideoProcessor {
    decoder: Arc<Mutex<VideoDecoder>>,
}

impl VideoProcessor {
    // Initialize with codec parameters
    pub fn initialize(codec_params: codec::Parameters) -> Result<Self, ffmpeg::Error> {
        let decoder = VideoDecoder::new(codec_params)?;
        Ok(VideoProcessor {
            decoder: Arc::new(Mutex::new(decoder)),
        })
    }

    // Feed a video packet to the processor
    pub fn feed_packet(&self, packet_data: &[u8]) -> Result<(), ffmpeg::Error> {
        let mut decoder = self.decoder.lock().unwrap();
        decoder.process_packet(packet_data)
    }

    // ... Other methods ...
}

pub fn save_frame_as_jpeg(frame: &frame::Video, filename: &str) -> Result<(), ffmpeg::Error> {
    let mut output_context = format::output(&filename)?;
    let global_header = output_context
        .format()
        .flags()
        .contains(format::flag::Flags::GLOBAL_HEADER);

    let encoder = encoder::find(codec::Id::MJPEG).ok_or(ffmpeg::Error::Bug)?;
    let mut encoder_context = codec::context::Context::new()?; // Change here
    encoder_context.set_codec(&encoder);

    encoder_context.set_height(frame.height());
    encoder_context.set_width(frame.width());
    encoder_context.set_format(frame.format());
    encoder_context.set_time_base((1, 30));
    if global_header {
        encoder_context.set_flags(codec::flag::Flags::GLOBAL_HEADER);
    }
    encoder_context.open(None)?;

    let mut stream = output_context.add_stream(Some(&encoder_context))?;
    output_context.write_header()?;

    let mut filtered_frame = frame.clone();
    filtered_frame.set_format(encoder_context.format());

    encoder_context.send_frame(Some(&filtered_frame))?;
    let mut packet = Packet::empty();
    while encoder_context.receive_packet(&mut packet).is_ok() {
        stream.write(&packet)?; // Change here
    }
    encoder_context.send_frame(None)?; // Send flush packet

    output_context.write_trailer()?;

    Ok(())
}
