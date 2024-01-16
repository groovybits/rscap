use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use std::fs::File;
use std::io::Write;
use std::sync::{Arc, Mutex};

pub struct VideoDecoder {
    pipeline: gst::Pipeline,
    appsrc: gst_app::AppSrc,
    appsink: gst_app::AppSink,
}

impl VideoDecoder {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        gst::init()?;

        let pipeline = gst::Pipeline::new();

        let appsrc = gst::ElementFactory::make("appsrc")
            .build()?
            .downcast::<gst_app::AppSrc>()
            .map_err(|_| Box::<dyn std::error::Error>::from("Failed to create appsrc element"))?;

        let decodebin = gst::ElementFactory::make("decodebin")
            .build()?
            .downcast::<gst_app::AppSrc>()
            .map_err(|_| {
                Box::<dyn std::error::Error>::from("Failed to create decodebin element")
            })?;
        // fails in add_many if not AppSrc???
        //let decodebin = gst::ElementFactory::make("decodebin").build().unwrap();

        let jpegenc = gst::ElementFactory::make("jpegenc")
            .build()?
            .downcast::<gst_app::AppSrc>()
            .map_err(|_| Box::<dyn std::error::Error>::from("Failed to create jpegenc element"))?;
        // fails in add_many if not AppSrc???

        let appsink = gst::ElementFactory::make("appsink")
            .build()?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| Box::<dyn std::error::Error>::from("Failed to create appsink element"))?;

        let elements = &[&appsrc, &decodebin, &jpegenc]; //, &appsink];
        pipeline.add_many(elements)?;
        //let elements = &[&appsrc, &decodebin];
        gst::Element::link_many(elements)?;
        jpegenc.link(&appsink)?;
        decodebin.connect_pad_added(move |_, src_pad| {
            let sink_pad = jpegenc.static_pad("sink").unwrap();
            if sink_pad.is_linked() {
                return;
            }
            src_pad.link(&sink_pad).expect("Failed to link pads");
        });

        let mpegts_caps = gst::Caps::builder("video/mpegts").build();
        let jpeg_caps = gst::Caps::builder("image/jpeg").build();

        appsrc.set_caps(Some(&mpegts_caps));
        appsrc.set_format(gst::Format::Time);

        appsink.set_caps(Some(&jpeg_caps));
        appsink.set_drop(true); // Drop old frames, only keep the latest

        // Handle new sample from appsink
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(|appsink| {
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Eos)?;

                    if let Some(map) = buffer.map_readable().ok() {
                        let mut file = File::create("frame.jpg").expect("Failed to create file");
                        file.write_all(map.as_slice())
                            .expect("Failed to write to file");
                    }

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        let bus = pipeline.bus().expect("Failed to get pipeline bus");
        for msg in bus.iter_timed(gst::ClockTime::NONE) {
            match msg.view() {
                gst::MessageView::Eos(..) => break,
                gst::MessageView::Error(err) => {
                    eprintln!("Error from {:?}: {:?}", err.src(), err.error());
                    break;
                }
                _ => (),
            }
        }

        Ok(VideoDecoder {
            pipeline,
            appsrc,
            appsink,
        })
    }

    pub fn listen_to_bus(&self) {
        let bus = self.pipeline.bus().expect("Failed to get pipeline bus");
        for msg in bus.iter_timed(gst::ClockTime::NONE) {
            match msg.view() {
                gst::MessageView::Eos(..) => break,
                gst::MessageView::Error(err) => {
                    eprintln!("Error from {:?}: {:?}", err.src(), err.error());
                    break;
                }
                _ => (),
            }
        }
    }

    pub fn process_packet(&self, packet_data: &[u8]) -> Result<(), gst::FlowError> {
        let buffer = gst::Buffer::from_slice(packet_data.to_vec());
        self.appsrc.push_buffer(buffer)?;
        Ok(())
    }

    pub fn start(&self) -> Result<(), gst::StateChangeError> {
        self.pipeline.set_state(gst::State::Playing)?;
        Ok(())
    }

    pub fn stop(&self) -> Result<(), gst::StateChangeError> {
        self.pipeline.set_state(gst::State::Null)?;
        Ok(())
    }
}

pub struct VideoProcessor {
    decoder: Arc<Mutex<VideoDecoder>>,
}

impl VideoProcessor {
    pub fn initialize() -> Result<Self, Box<dyn std::error::Error>> {
        let decoder = VideoDecoder::new()?;
        Ok(VideoProcessor {
            decoder: Arc::new(Mutex::new(decoder)),
        })
    }
    pub fn feed_packet(&self, packet_data: &[u8]) -> Result<(), gst::FlowError> {
        let decoder = self.decoder.lock().unwrap();
        decoder.process_packet(packet_data)
    }

    pub fn start(&self) -> Result<(), gst::StateChangeError> {
        let decoder = self.decoder.lock().unwrap();
        decoder.start()
    }

    pub fn stop(&self) -> Result<(), gst::StateChangeError> {
        let decoder = self.decoder.lock().unwrap();
        decoder.stop()
    }
}
