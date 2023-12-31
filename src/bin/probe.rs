/*
 * rscap: probe.rs - Rust Stream Capture with pcap, output json stats to ZeroMQ
 *
 * Written in 2024 by Chris Kennedy (C) LTN Global
 *
 * License: LGPL v2.1
 *
 */

use async_zmq;
#[cfg(feature = "dpdk_enabled")]
use capsule::config::{load_config, DPDKConfig};
#[cfg(feature = "dpdk_enabled")]
use capsule::dpdk;
use clap::Parser;
use futures::stream::StreamExt;
use lazy_static::lazy_static;
use log::{debug, error, info};
use pcap::{Active, Capture, Device, PacketCodec};
use rtp::RtpReader;
use rtp_rs as rtp;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    error::Error as StdError,
    fmt,
    net::{IpAddr, Ipv4Addr, UdpSocket},
    sync::atomic::{AtomicBool, Ordering},
    sync::Arc,
    sync::Mutex,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::mpsc::{self};
use zmq::PUB;
//use capnp;
/*use capnp::message::{Builder, HeapAllocator};*/
// Include the generated paths for the Cap'n Proto schema
include!("../stream_data_capnp.rs");

// Define your custom PacketCodec
pub struct BoxCodec;

impl PacketCodec for BoxCodec {
    type Item = Box<[u8]>;

    fn decode(&mut self, packet: pcap::Packet) -> Self::Item {
        packet.data.into()
    }
}

// constant for PAT PID
const PAT_PID: u16 = 0;
const TS_PACKET_SIZE: usize = 188;

// global variable to store the MpegTS PID Map (initially empty)
lazy_static! {
    static ref PID_MAP: Mutex<HashMap<u16, Arc<StreamData>>> = Mutex::new(HashMap::new());
}

struct PatEntry {
    program_number: u16,
    pmt_pid: u16,
}

struct PmtEntry {
    stream_pid: u16,
    stream_type: u8, // Stream type (e.g., 0x02 for MPEG video)
}

struct Pmt {
    entries: Vec<PmtEntry>,
}

#[derive(Clone, PartialEq)]
enum Codec {
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
struct StreamData {
    pid: u16,
    pmt_pid: u16,
    program_number: u16,
    stream_type: String, // "video", "audio", "text"
    continuity_counter: u8,
    timestamp: u64,
    bitrate: u32,
    bitrate_max: u32,
    bitrate_min: u32,
    bitrate_avg: u32,
    iat: u64,
    iat_max: u64,
    iat_min: u64,
    iat_avg: u64,
    error_count: u32,
    last_arrival_time: u64,
    start_time: u64, // field for start time
    total_bits: u64, // field for total bits
    count: u32,      // field for count
    #[serde(skip)]
    packet: Arc<Vec<u8>>, // The actual MPEG-TS packet data
    packet_start: usize, // Offset into the data
    packet_len: usize, // Offset into the data
    // SMPTE 2110 fields
    rtp_timestamp: u32,
    rtp_payload_type: u8,
    rtp_payload_type_name: String,
    rtp_line_number: u16,
    rtp_line_offset: u16,
    rtp_line_length: u16,
    rtp_field_id: u8,
    rtp_line_continuation: u8,
    rtp_extended_sequence_number: u16,
}

impl Clone for StreamData {
    fn clone(&self) -> Self {
        StreamData {
            pid: self.pid,
            pmt_pid: self.pmt_pid,
            program_number: self.program_number,
            stream_type: self.stream_type.clone(),
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
            last_arrival_time: self.last_arrival_time,
            start_time: self.start_time,
            total_bits: self.total_bits,
            count: self.count,
            packet: Arc::new(Vec::new()), // Initialize as empty with Arc
            packet_start: 0,
            packet_len: 0,
            rtp_timestamp: self.rtp_timestamp,
            rtp_payload_type: self.rtp_payload_type,
            rtp_payload_type_name: self.rtp_payload_type_name.clone(),
            rtp_line_number: self.rtp_line_number,
            rtp_line_offset: self.rtp_line_offset,
            rtp_line_length: self.rtp_line_length,
            rtp_field_id: self.rtp_field_id,
            rtp_line_continuation: self.rtp_line_continuation,
            rtp_extended_sequence_number: self.rtp_extended_sequence_number,
        }
    }
}

// StreamData implementation
impl StreamData {
    fn new(
        packet: Arc<Vec<u8>>,
        packet_start: usize,
        packet_len: usize,
        pid: u16,
        stream_type: String,
        start_time: u64,
        timestamp: u64,
        continuity_counter: u8,
    ) -> Self {
        let last_arrival_time = current_unix_timestamp_ms().unwrap_or(0);
        StreamData {
            pid,
            pmt_pid: 0xFFFF,
            program_number: 0,
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
            last_arrival_time,
            start_time,    // Initialize start time
            total_bits: 0, // Initialize total bits
            count: 0,      // Initialize count
            packet: packet,
            packet_start: packet_start,
            packet_len: packet_len,
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
        }
    }
    // set RTP fields
    fn set_rtp_fields(
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
    fn update_stream_type(&mut self, stream_type: String) {
        self.stream_type = stream_type;
    }
    fn increment_error_count(&mut self, error_count: u32) {
        self.error_count += error_count;
    }
    fn increment_count(&mut self, count: u32) {
        self.count += count;
    }
    fn set_continuity_counter(&mut self, continuity_counter: u8) {
        // check for continuity continuous increment and wrap around from 0 to 15
        let previous_continuity_counter = self.continuity_counter;
        self.continuity_counter = continuity_counter & 0x0F;
        // check if we incremented without loss
        if self.continuity_counter != previous_continuity_counter + 1
            && self.continuity_counter != previous_continuity_counter
        {
            // check if we wrapped around from 15 to 0
            if self.continuity_counter == 0 {
                // check if previous value was 15
                if previous_continuity_counter == 15 {
                    // no loss
                    return;
                }
            }
            // loss
            self.increment_error_count(1);
            error!(
                "Continuity Counter Error: PID: {} Previous: {} Current: {}",
                self.pid, previous_continuity_counter, self.continuity_counter
            );
        }
        self.continuity_counter = continuity_counter;
    }
    fn update_stats(&mut self, packet_size: usize, arrival_time: u64) {
        let bits = packet_size as u64 * 8; // Convert bytes to bits

        // Elapsed time in milliseconds
        let elapsed_time_ms = arrival_time - self.start_time;

        if elapsed_time_ms > 0 {
            let elapsed_time_sec = elapsed_time_ms as f64 / 1000.0;
            self.bitrate = (self.total_bits as f64 / elapsed_time_sec) as u32;

            // Bitrate max
            if self.bitrate > self.bitrate_max {
                self.bitrate_max = self.bitrate;
            }

            // Bitrate min
            if self.bitrate < self.bitrate_min {
                self.bitrate_min = self.bitrate;
            }

            // Bitrate avg
            self.bitrate_avg = (self.bitrate_avg + self.bitrate) / 2;
        }

        self.total_bits += bits; // Accumulate total bits

        // IAT calculation remains the same
        let iat = arrival_time - self.last_arrival_time;
        self.iat = iat;

        // IAT max
        if iat > self.iat_max {
            self.iat_max = iat;
        }

        // IAT min
        if iat < self.iat_min {
            self.iat_min = iat;
        }

        // IAT avg
        self.iat_avg = (self.iat_avg + iat) / 2;

        self.last_arrival_time = arrival_time;
    }
}

#[derive(Serialize, Deserialize)]
struct Tr101290Errors {
    // p1 errors
    ts_sync_byte_errors: u32,
    sync_byte_errors: u32,
    continuity_counter_errors: u32,
    pat_errors: u32,
    pmt_errors: u32,
    pid_map_errors: u32,
    // p2 errors
    transport_error_indicator_errors: u32,
    crc_errors: u32,
    pcr_repetition_errors: u32,
    pcr_discontinuity_indicator_errors: u32,
    pcr_accuracy_errors: u32,
    pts_errors: u32,
    cat_errors: u32,
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
    fn new() -> Self {
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
fn tr101290_p1_check(packet: &[u8], errors: &mut Tr101290Errors) {
    // p1
    if packet[0] != 0x47 {
        errors.sync_byte_errors += 1;
    }

    // TODO: ... other checks, updating the respective counters ...
}

// TR 101 290 Priority 2 Check
fn tr101290_p2_check(packet: &[u8], errors: &mut Tr101290Errors) {
    // p2

    if (packet[1] & 0x80) != 0 {
        errors.transport_error_indicator_errors += 1;
    }
    // TODO: ... other checks, updating the respective counters ...
}

/*
// Function to convert StreamData to a Cap'n Proto message
fn stream_data_to_capnp(stream_data: &StreamData) -> capnp::Result<Builder<HeapAllocator>> {
    let mut message = Builder::new_default();
    {
        let mut stream_data_msg = message.init_root::<stream_data_capnp::stream_data::Builder>();

        stream_data_msg.set_pid(stream_data.pid);
        stream_data_msg.set_pmt_pid(stream_data.pmt_pid);
        stream_data_msg.set_program_number(stream_data.program_number);
        stream_data_msg.set_stream_type(&stream_data.stream_type);
        // ... Continue setting other fields
    }

    Ok(message)
}

// Function to convert a Cap'n Proto message back to StreamData
fn capnp_to_stream_data(reader: stream_data_capnp::stream_data::Reader) -> capnp::Result<StreamData> {
    let stream_data = StreamData {
        pid: reader.get_pid()?,
        pmt_pid: reader.get_pmt_pid()?,
        program_number: reader.get_program_number()?,
        stream_type: reader.get_stream_type()?.to_string(),
        // ... Continue setting other fields
        packet: Arc::new(Vec::new()), // Initialize packet as empty
        packet_start: 0,
        packet_len: 0,
        // ... Initialize other fields as required
    };

    Ok(stream_data)
}
*/

/*

// Define a struct to hold information about the current video frame
struct VideoFrame {
    packets: Vec<StreamData>,
    is_complete: bool,
}

impl VideoFrame {
    fn new() -> Self {
        VideoFrame {
            packets: Vec::new(),
            is_complete: false,
        }
    }

    fn add_packet(&mut self, stream_data: StreamData) {
        self.packets.push(stream_data);
    }

    fn clear(&mut self) {
        self.packets.clear();
        self.is_complete = false;
    }
}
*/

// Invoke this function for each MPEG-TS packet
fn process_packet(
    stream_data_packet: &mut StreamData,
    errors: &mut Tr101290Errors,
    is_mpegts: bool,
    pmt_pid: u16,
) {
    let packet: &[u8] = &stream_data_packet.packet[stream_data_packet.packet_start
        ..stream_data_packet.packet_start + stream_data_packet.packet_len];
    tr101290_p1_check(packet, errors);
    tr101290_p2_check(packet, errors);

    let pid = stream_data_packet.pid;
    let arrival_time = current_unix_timestamp_ms().unwrap_or(0);

    let mut pid_map = PID_MAP.lock().unwrap();

    // TODO: high debug level output, may need a flag specific to this dump
    //info!("PID Map Contents: {:#?}", pid_map);

    // Check if the PID map already has an entry for this PID
    match pid_map.get_mut(&pid) {
        Some(stream_data_arc) => {
            // Existing StreamData instance found, update it
            let mut stream_data = Arc::clone(stream_data_arc);
            Arc::make_mut(&mut stream_data).update_stats(packet.len(), arrival_time);
            Arc::make_mut(&mut stream_data).increment_count(1);
            if stream_data.pid != 0x1FFF && is_mpegts {
                Arc::make_mut(&mut stream_data)
                    .set_continuity_counter(stream_data_packet.continuity_counter);
            }
            let uptime = arrival_time - stream_data.start_time;

            // print out each field of structure similar to json but not wrapping into json
            debug!("STATUS::PACKET:MODIFY[{}] pid: {} stream_type: {} bitrate: {} bitrate_max: {} bitrate_min: {} bitrate_avg: {} iat: {} iat_max: {} iat_min: {} iat_avg: {} errors: {} continuity_counter: {} timestamp: {} uptime: {}", stream_data.pid, stream_data.pid, stream_data.stream_type, stream_data.bitrate, stream_data.bitrate_max, stream_data.bitrate_min, stream_data.bitrate_avg, stream_data.iat, stream_data.iat_max, stream_data.iat_min, stream_data.iat_avg, stream_data.error_count, stream_data.continuity_counter, stream_data.timestamp, uptime);

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
            stream_data_packet.error_count = stream_data.error_count;
            stream_data_packet.last_arrival_time = stream_data.last_arrival_time;
            stream_data_packet.total_bits = stream_data.total_bits;
            stream_data_packet.count = stream_data.count;

            // write the stream_data back to the pid_map with modified values
            pid_map.insert(pid, stream_data);
        }
        None => {
            // No StreamData instance found for this PID, possibly no PMT yet
            if pmt_pid != 0xFFFF {
                debug!("ProcessPacket: New PID {} Found, adding to PID map.", pid);
            } else {
                // PMT packet not found yet, add the stream_data_packet to the pid_map
                let mut stream_data = Arc::new(StreamData::new(
                    Arc::new(Vec::new()), // Ensure packet_data is Arc<Vec<u8>>
                    0,
                    0,
                    stream_data_packet.pid,
                    stream_data_packet.stream_type.clone(),
                    stream_data_packet.start_time,
                    stream_data_packet.timestamp,
                    stream_data_packet.continuity_counter,
                ));
                Arc::make_mut(&mut stream_data).update_stats(packet.len(), arrival_time);

                // print out each field of structure similar to json but not wrapping into json
                info!("STATUS::PACKET:ADD[{}] pid: {} stream_type: {} bitrate: {} bitrate_max: {} bitrate_min: {} bitrate_avg: {} iat: {} iat_max: {} iat_min: {} iat_avg: {} errors: {} continuity_counter: {} timestamp: {} uptime: {}", stream_data.pid, stream_data.pid, stream_data.stream_type, stream_data.bitrate, stream_data.bitrate_max, stream_data.bitrate_min, stream_data.bitrate_avg, stream_data.iat, stream_data.iat_max, stream_data.iat_min, stream_data.iat_avg, stream_data.error_count, stream_data.continuity_counter, stream_data.timestamp, 0);

                pid_map.insert(pid, stream_data);
            }
        }
    }
}

// Function to get the current Unix timestamp in milliseconds
fn current_unix_timestamp_ms() -> Result<u64, &'static str> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .map_err(|_| "System time is before the UNIX epoch")
}

// Implement a function to extract PID from a packet
fn extract_pid(packet: &[u8]) -> u16 {
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
struct PmtInfo {
    pid: u16,
    packet: Vec<u8>,
}

// Helper function to parse PAT and update global PAT packet storage
fn parse_and_store_pat(packet: &[u8]) -> PmtInfo {
    let pat_entries = parse_pat(packet);
    let mut pmt_info = PmtInfo {
        pid: 0xFFFF,
        packet: Vec::new(),
    };
    pmt_info.packet = packet.to_vec();

    // Assuming there's only one program for simplicity, update PMT PID
    if let Some(first_entry) = pat_entries.first() {
        pmt_info.pid = first_entry.pmt_pid;
    }
    pmt_info
}

fn parse_pat(packet: &[u8]) -> Vec<PatEntry> {
    let mut entries = Vec::new();

    // Check for minimum packet size
    if packet.len() < TS_PACKET_SIZE {
        return entries;
    }

    // Check if Payload Unit Start Indicator (PUSI) is set
    let pusi = (packet[1] & 0x40) != 0;
    if !pusi {
        // If Payload Unit Start Indicator is not set, this packet does not start a new PAT
        return entries;
    }

    let adaptation_field_control = (packet[3] & 0x30) >> 4;
    let mut offset = 4; // start after TS header

    // Check for adaptation field and skip it
    if adaptation_field_control == 0x02 || adaptation_field_control == 0x03 {
        let adaptation_field_length = packet[4] as usize;
        offset += 1 + adaptation_field_length; // +1 for the length byte itself
    }

    // Pointer field indicates the start of the PAT section
    let pointer_field = packet[offset] as usize;
    offset += 1 + pointer_field; // Skip pointer field

    // Now, 'offset' points to the start of the PAT section
    while offset + 4 <= packet.len() {
        let program_number = ((packet[offset] as u16) << 8) | (packet[offset + 1] as u16);
        let pmt_pid = (((packet[offset + 2] as u16) & 0x1F) << 8) | (packet[offset + 3] as u16);

        // Only add valid entries (non-zero program_number and pmt_pid)
        if program_number != 0 && pmt_pid != 0 && pmt_pid < 0x1FFF && program_number < 100 {
            entries.push(PatEntry {
                program_number,
                pmt_pid,
            });
        }

        debug!(
            "ParsePAT: Program Number: {} PMT PID: {}",
            program_number, pmt_pid
        );

        offset += 4; // Move to the next PAT entry
    }

    entries
}

fn parse_pmt(packet: &[u8]) -> Pmt {
    let mut entries = Vec::new();
    let program_number = ((packet[8] as u16) << 8) | (packet[9] as u16);

    // Calculate the starting position for stream entries
    let section_length = (((packet[6] as usize) & 0x0F) << 8) | packet[7] as usize;
    let program_info_length = (((packet[15] as usize) & 0x0F) << 8) | packet[16] as usize;
    let mut i = 17 + program_info_length; // Starting index of the first stream in the PMT

    debug!(
        "ParsePMT: Program Number: {} PMT PID: {} starting at position {}",
        program_number,
        extract_pid(packet),
        i
    );
    while i + 5 <= packet.len() && i < 17 + section_length - 4 {
        let stream_type = packet[i];
        let stream_pid = (((packet[i + 1] as u16) & 0x1F) << 8) | (packet[i + 2] as u16);
        let es_info_length = (((packet[i + 3] as usize) & 0x0F) << 8) | packet[i + 4] as usize;
        i += 5 + es_info_length; // Update index to point to next stream's info

        entries.push(PmtEntry {
            stream_pid,
            stream_type,
        });
        debug!(
            "ParsePMT: Stream PID: {}, Stream Type: {}",
            stream_pid, stream_type
        );
    }

    Pmt { entries }
}

// Helper function to identify the video PID from the stored PAT packet and return the PID and codec
fn identify_video_pid(pmt_packet: &[u8]) -> Option<(u16, Codec)> {
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

// Use the stored PAT packet
fn update_pid_map(pmt_packet: &[u8], last_pat_packet: &[u8]) {
    let mut pid_map = PID_MAP.lock().unwrap();

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

        // Ensure the current PMT packet matches the PMT PID from the PAT
        if extract_pid(pmt_packet) == pmt_pid {
            let pmt = parse_pmt(pmt_packet);

            for pmt_entry in pmt.entries.iter() {
                debug!(
                    "UpdatePIDmap: Processing PMT PID: {} for Stream PID: {} Type {}",
                    pmt_pid, pmt_entry.stream_pid, pmt_entry.stream_type
                );

                let stream_pid = pmt_entry.stream_pid;
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
                        timestamp,
                        timestamp,
                        0,
                    ));
                    // update stream_data stats
                    Arc::make_mut(&mut stream_data).update_stats(pmt_packet.len(), timestamp);

                    // print out each field of structure similar to json but not wrapping into json
                    info!("STATUS::STREAM:CREATE[{}] pid: {} stream_type: {} bitrate: {} bitrate_max: {} bitrate_min: {} bitrate_avg: {} iat: {} iat_max: {} iat_min: {} iat_avg: {} errors: {} continuity_counter: {} timestamp: {} uptime: {}", stream_data.pid, stream_data.pid, stream_data.stream_type, stream_data.bitrate, stream_data.bitrate_max, stream_data.bitrate_min, stream_data.bitrate_avg, stream_data.iat, stream_data.iat_max, stream_data.iat_min, stream_data.iat_avg, stream_data.error_count, stream_data.continuity_counter, stream_data.timestamp, 0);

                    pid_map.insert(stream_pid, stream_data);
                } else {
                    // get the stream data so we can update it
                    let stream_data_arc = pid_map.get_mut(&stream_pid).unwrap();
                    let mut stream_data = Arc::clone(stream_data_arc);

                    // update the stream type
                    Arc::make_mut(&mut stream_data).update_stream_type(stream_type.to_string());

                    // print out each field of structure similar to json but not wrapping into json
                    debug!("STATUS::STREAM:UPDATE[{}] pid: {} stream_type: {} bitrate: {} bitrate_max: {} bitrate_min: {} bitrate_avg: {} iat: {} iat_max: {} iat_min: {} iat_avg: {} errors: {} continuity_counter: {} timestamp: {} uptime: {}", stream_data.pid, stream_data.pid, stream_data.stream_type, stream_data.bitrate, stream_data.bitrate_max, stream_data.bitrate_min, stream_data.bitrate_avg, stream_data.iat, stream_data.iat_max, stream_data.iat_min, stream_data.iat_avg, stream_data.error_count, stream_data.continuity_counter, stream_data.timestamp, 0);

                    // write the stream_data back to the pid_map with modified values
                    pid_map.insert(stream_pid, stream_data);
                }
            }
        } else {
            error!("UpdatePIDmap: Skipping PMT PID: {} as it does not match with current PMT packet PID", pmt_pid);
        }
    }
}

fn determine_stream_type(pid: u16) -> String {
    let pid_map = PID_MAP.lock().unwrap();

    // check if pid already is mapped, if so return the stream type already stored
    if let Some(stream_data) = pid_map.get(&pid) {
        return stream_data.stream_type.clone();
    }

    pid_map
        .get(&pid)
        .map(|stream_data| stream_data.stream_type.clone())
        .unwrap_or_else(|| "unknown".to_string())
}

// Define a custom error for when the target device is not found
#[derive(Debug)]
struct DeviceNotFoundError;

impl std::fmt::Display for DeviceNotFoundError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Target device not found")
    }
}

impl StdError for DeviceNotFoundError {}

#[cfg(all(feature = "dpdk_enabled", target_os = "linux"))]
fn init_dpdk(port_id: u16) -> Result<dpdk::eal::Port, Box<dyn std::error::Error>> {
    // DPDK initialization logic here
    let config = load_config()?;
    info!("DPDK config: {:?} setup on port {}", config, port_id);
    dpdk::eal::init(config)?;

    // Configure and start the network interface
    let port = dpdk::eal::Port::new(port_id)?;
    port.configure()?;
    port.start()?;

    // Set promiscuous mode if needed
    if promiscuous_mode {
        port.set_promiscuous(true)?;
    }

    info!("DPDK port {} initialized", port_id);

    Ok(port)
}

// Placeholder for non-Linux or DPDK disabled builds
#[cfg(not(all(feature = "dpdk_enabled", target_os = "linux")))]
fn init_dpdk(_port_id: u16) -> Result<(), Box<dyn std::error::Error>> {
    Err("DPDK is not supported on this OS".into())
}

fn init_pcap(
    source_device: &str,
    use_wireless: bool,
    promiscuous: bool,
    read_time_out: i32,
    read_size: i32,
    immediate_mode: bool,
    buffer_size: i64,
    source_protocol: &str,
    source_port: i32,
    source_ip: &str,
) -> Result<(Capture<Active>, UdpSocket), Box<dyn StdError>> {
    let devices = Device::list().map_err(|e| Box::new(e) as Box<dyn StdError>)?;
    debug!("init_pcap: devices: {:?}", devices);
    info!("init_pcap: specified source_device: {}", source_device);

    // Different handling for Linux and non-Linux systems
    #[cfg(target_os = "linux")]
    let target_device = devices
        .into_iter()
        .find(|d| d.name == source_device || source_device.is_empty())
        .ok_or_else(|| Box::new(DeviceNotFoundError) as Box<dyn StdError>)?;

    #[cfg(not(target_os = "linux"))]
    let target_device = devices
        .into_iter()
        .find(|d| {
            (d.name == source_device || source_device.is_empty())
                && d.flags.is_up()
                && !d.flags.is_loopback()
                && d.flags.is_running()
                && (!d.flags.is_wireless() || use_wireless)
        })
        .ok_or_else(|| Box::new(DeviceNotFoundError) as Box<dyn StdError>)?;

    // Get the IP address of the target device
    let interface_addr = target_device
        .addresses
        .iter()
        .find_map(|addr| match addr.addr {
            IpAddr::V4(ipv4_addr) => Some(ipv4_addr),
            _ => None,
        })
        .ok_or_else(|| "No valid IPv4 address found for target device")?;

    let multicast_addr = source_ip
        .parse::<Ipv4Addr>()
        .expect("Invalid IP address format for source_ip");

    info!(
        "init_pcap: UDP Socket Binding to interface {} with Join IGMP Multicast for address:port udp://{}:{}.",
        interface_addr, multicast_addr, source_port
    );

    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| Box::new(e) as Box<dyn StdError>)?;
    socket
        .join_multicast_v4(&multicast_addr, &interface_addr)
        .map_err(|e| Box::new(e) as Box<dyn StdError>)?;

    let source_host_and_port = format!(
        "{} dst port {} and ip dst host {}",
        source_protocol, source_port, source_ip
    );

    let cap = Capture::from_device(target_device.clone())
        .map_err(|e| Box::new(e) as Box<dyn StdError>)?
        .promisc(promiscuous)
        .timeout(read_time_out)
        .snaplen(read_size)
        .immediate_mode(immediate_mode)
        .buffer_size(buffer_size as i32)
        .open()
        .map_err(|e| Box::new(e) as Box<dyn StdError>)?;

    info!(
        "init_pcap: set non-blocking mode on capture device {}",
        target_device.name
    );

    let mut cap = cap
        .setnonblock()
        .map_err(|e| Box::new(e) as Box<dyn StdError>)?;

    info!(
        "init_pcap: set filter for {} on capture device {}",
        source_host_and_port, target_device.name
    );

    cap.filter(&source_host_and_port, true)
        .map_err(|e| Box::new(e) as Box<dyn StdError>)?;

    info!(
        "init_pcap: capture device {} successfully initialized",
        target_device.name
    );

    Ok((cap, socket))
}

/// RScap Probe Configuration
#[derive(Parser, Debug)]
#[clap(
    author = "Chris Kennedy",
    version = "1.1",
    about = "RsCap Probe for ZeroMQ output of MPEG-TS and SMPTE 2110 streams from pcap."
)]
struct Args {
    /// Sets the batch size
    #[clap(long, env = "BATCH_SIZE", default_value_t = 7)]
    batch_size: usize,

    /// Sets the payload offset
    #[clap(long, env = "PAYLOAD_OFFSET", default_value_t = 42)]
    payload_offset: usize,

    /// Sets the packet size
    #[clap(long, env = "PACKET_SIZE", default_value_t = 188)]
    packet_size: usize,

    /// Sets the read timeout
    #[clap(long, env = "READ_TIME_OUT", default_value_t = 60_000)]
    read_time_out: i32,

    /// Sets the target port
    #[clap(long, env = "TARGET_PORT", default_value_t = 5_556)]
    target_port: i32,

    /// Sets the target IP
    #[clap(long, env = "TARGET_IP", default_value = "127.0.0.1")]
    target_ip: String,

    /// Sets the source device
    #[clap(long, env = "SOURCE_DEVICE", default_value = "")]
    source_device: String,

    /// Sets the source IP
    #[clap(long, env = "SOURCE_IP", default_value = "224.0.0.200")]
    source_ip: String,

    /// Sets the source protocol
    #[clap(long, env = "SOURCE_PROTOCOL", default_value = "udp")]
    source_protocol: String,

    /// Sets the source port
    #[clap(long, env = "SOURCE_PORT", default_value_t = 10_000)]
    source_port: i32,

    /// Sets the debug mode
    #[clap(long, env = "DEBUG", default_value_t = false)]
    debug_on: bool,

    /// Sets the silent mode
    #[clap(long, env = "SILENT", default_value_t = false)]
    silent: bool,

    /// Sets if wireless is used
    #[clap(long, env = "USE_WIRELESS", default_value_t = false)]
    use_wireless: bool,

    /// Sets if JSON header should be sent
    #[clap(long, env = "SEND_JSON_HEADER", default_value_t = false)]
    send_json_header: bool,

    /// Sets if Raw Stream should be sent
    #[clap(long, env = "SEND_RAW_STREAM", default_value_t = false)]
    send_raw_stream: bool,

    /// number of packets to capture
    #[clap(long, env = "PACKET_COUNT", default_value_t = 0)]
    packet_count: u64,

    /// Turn off progress output dots
    #[clap(long, env = "NO_PROGRESS", default_value_t = false)]
    no_progress: bool,

    /// Turn off ZeroMQ send
    #[clap(long, env = "NO_ZMQ", default_value_t = false)]
    no_zmq: bool,

    /// Force smpte2110 mode
    #[clap(long, env = "SMPT2110", default_value_t = false)]
    smpte2110: bool,

    /// Use promiscuous mode
    #[clap(long, env = "PROMISCUOUS", default_value_t = false)]
    promiscuous: bool,

    /// Show the TR101290 p1, p2 and p3 errors if any
    #[clap(long, env = "SHOW_TR101290", default_value_t = false)]
    show_tr101290: bool,

    /// Sets the pcap buffer size
    #[clap(long, env = "BUFFER_SIZE", default_value_t = 1 * 1_358 * 1_000)]
    buffer_size: usize,

    /// PCAP immediate mode
    #[clap(long, env = "IMMEDIATE_MODE", default_value_t = false)]
    immediate_mode: bool,

    /// PCAP output capture stats mode
    #[clap(long, env = "PCAP_STATS", default_value_t = false)]
    pcap_stats: bool,

    ///  MPSC Channel Size for ZeroMQ
    #[clap(long, env = "PCAP_CHANNEL_SIZE", default_value_t = 1_000)]
    pcap_channel_size: usize,

    /// MPSC Channel Size for PCAP
    #[clap(long, env = "ZMQ_CHANNEL_SIZE", default_value_t = 1_000)]
    zmq_channel_size: usize,

    /// DPDK enable
    #[clap(long, env = "DPDK", default_value_t = false)]
    dpdk: bool,

    /// DPDK Port ID
    #[clap(long, env = "DPDK_PORT_ID", default_value_t = 0)]
    dpdk_port_id: u16,
}

// MAIN Function
#[tokio::main]
async fn main() {
    println!("RsCap Probe for ZeroMQ output of MPEG-TS and SMPTE 2110 streams from pcap.");

    dotenv::dotenv().ok(); // read .env file

    let args = Args::parse();

    // Use the parsed arguments directly
    let batch_size = args.batch_size;
    let payload_offset = args.payload_offset;
    let mut packet_size = args.packet_size;
    let read_time_out = args.read_time_out;
    let target_port = args.target_port;
    let target_ip = args.target_ip;
    let source_device = args.source_device;
    let source_ip = args.source_ip;
    let source_protocol = args.source_protocol;
    let source_port = args.source_port;
    let debug_on = args.debug_on;
    let silent = args.silent;
    let use_wireless = args.use_wireless;
    let send_json_header = args.send_json_header;
    let send_raw_stream = args.send_raw_stream;
    let packet_count = args.packet_count;
    let no_progress = args.no_progress;
    let no_zmq = args.no_zmq;
    let promiscuous = args.promiscuous;
    let show_tr101290 = args.show_tr101290;
    let mut buffer_size = args.buffer_size as i64;
    let mut immediate_mode = args.immediate_mode;
    let pcap_stats = args.pcap_stats;
    let mut pcap_channel_size = args.pcap_channel_size;
    let mut zmq_channel_size = args.zmq_channel_size;
    #[cfg(all(feature = "dpdk_enabled", target_os = "linux"))]
    let use_dpdk = args.dpdk;

    if args.smpte2110 {
        // --batch-size 1 --buffer-size 10000000000 --pcap-channel-size 100000 --zmq-channel-size 10000 --packet-size 1208 --immediate-mode
        buffer_size = 10000000000; // set buffer size to 10GB for smpte2110
        pcap_channel_size = 100_000; // set pcap channel size to 100000 for smpte2110
        zmq_channel_size = 10_000; // set zmq channel size to 10000 for smpte2110
        packet_size = 1208; // set packet size to 1208 for smpte2110
        immediate_mode = true; // set immediate mode to true for smpte2110
    }

    if silent {
        // set log level to error
        std::env::set_var("RUST_LOG", "error");
    }

    // calculate read size based on batch size and packet size
    let read_size: i32 = (packet_size as i32 * batch_size as i32) + payload_offset as i32; // pcap read size

    let mut is_mpegts = true; // Default to true, update based on actual packet type

    // Initialize logging
    let _ = env_logger::try_init();

    let (ptx, mut prx) = mpsc::channel::<Arc<Vec<u8>>>(pcap_channel_size);

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    // Spawn a new thread for packet capture
    let capture_task = if cfg!(feature = "dpdk_enabled") && args.dpdk {
        // DPDK is enabled
        tokio::spawn(async move {
            // TODO: DPDK initialization logic goes here");
            error!("DPDK is not yet implemented");
            // Implement DPDK initialization logic here
            match init_dpdk(args.dpdk_port_id) {
                Ok(()) => {
                    let mut count = 0;
                    // DPDK packet processing loop
                    while running_clone.load(Ordering::SeqCst) {
                        // Fetch and process packets using DPDK
                        // ...

                        count += 1;
                        if debug_on {
                            // Print stats or debug information
                            println!("DPDK packet count: {}", count);
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to initialize DPDK: {}", e);
                }
            }
        })
    } else {
        tokio::spawn(async move {
            // initialize the pcap
            let (cap, _socket) = init_pcap(
                &source_device,
                use_wireless,
                promiscuous,
                read_time_out,
                read_size,
                immediate_mode,
                buffer_size as i64,
                &source_protocol,
                source_port,
                &source_ip,
            )
            .expect("Failed to initialize pcap");

            // Create a PacketStream from the Capture
            let mut stream = cap.stream(BoxCodec).unwrap();
            let mut count = 0;

            let mut stats_last_sent_ts = Instant::now();

            while running_clone.load(Ordering::SeqCst) {
                while let Some(packet) = stream.next().await {
                    match packet {
                        Ok(data) => {
                            count += 1;
                            let packet_data = Arc::new(data.to_vec());
                            ptx.send(packet_data).await.unwrap();
                            let current_ts = Instant::now();
                            if pcap_stats
                                && ((current_ts.duration_since(stats_last_sent_ts).as_secs() >= 30)
                                    || count == 1)
                            {
                                stats_last_sent_ts = current_ts;
                                let stats = stream.capture_mut().stats().unwrap();
                                println!(
                                    "#{} Current stats: Received: {}, Dropped: {}, Interface Dropped: {}",
                                    count, stats.received, stats.dropped, stats.if_dropped
                                );
                            }
                        }
                        Err(e) => {
                            // Print error and information about it
                            error!("PCap Capture Error occurred: {}", e);
                            if e == pcap::Error::TimeoutExpired {
                                // Timeout expired, continue and try again
                                continue;
                            } else {
                                // Exit the loop if an error occurs
                                running_clone.store(false, Ordering::SeqCst);
                                break;
                            }
                        }
                    }
                }
                if debug_on {
                    let stats = stream.capture_mut().stats().unwrap();
                    println!(
                        "Current stats: Received: {}, Dropped: {}, Interface Dropped: {}",
                        stats.received, stats.dropped, stats.if_dropped
                    );
                }
            }

            let stats = stream.capture_mut().stats().unwrap();
            println!("Packet capture statistics:");
            println!("Received: {}", stats.received);
            println!("Dropped: {}", stats.dropped);
            println!("Interface Dropped: {}", stats.if_dropped);
        })
    };

    // Setup channel for passing data between threads
    let (tx, mut rx) = mpsc::channel::<Vec<StreamData>>(zmq_channel_size);

    // Spawn a new thread for ZeroMQ communication
    let zmq_thread = tokio::spawn(async move {
        let context = async_zmq::Context::new();
        let publisher = context.socket(PUB).unwrap();
        let source_port_ip = format!("tcp://{}:{}", target_ip, target_port);
        publisher.bind(&source_port_ip).unwrap();

        info!("ZeroMQ publisher startup {}", source_port_ip);

        while let Some(mut batch) = rx.recv().await {
            for stream_data in batch.iter() {
                //debug!("Batch processing {} packets", batch.len());
                let packet_slice = &stream_data.packet
                    [stream_data.packet_start..stream_data.packet_start + stream_data.packet_len];

                // Serialize metadata only if necessary
                let metadata_msg = if send_json_header {
                    let cloned_stream_data = stream_data.clone(); // Clone avoids copying the Arc<Vec<u8>>
                    let metadata = serde_json::to_string(&cloned_stream_data).unwrap();

                    Some(zmq::Message::from(metadata.as_bytes()))
                } else {
                    None
                };

                if send_json_header && send_raw_stream {
                    if let Some(meta_msg) = metadata_msg {
                        // Send metadata as the first message
                        publisher.send(meta_msg, zmq::SNDMORE).unwrap();

                        // Send packet data as the second message
                        let packet_msg = zmq::Message::from(packet_slice);
                        publisher.send(packet_msg, 0).unwrap();
                    }
                } else if send_json_header {
                    // Send metadata only if send_json_header is false
                    if let Some(meta_msg) = metadata_msg {
                        publisher.send(meta_msg, 0).unwrap();
                    }
                } else if send_raw_stream {
                    // Send packet data only if send_raw_stream is ftrue
                    let packet_msg = zmq::Message::from(packet_slice);
                    publisher.send(packet_msg, 0).unwrap();
                }
            }
            batch.clear();
        }
    });

    // Perform TR 101 290 checks
    let mut tr101290_errors = Tr101290Errors::new();

    // start time
    let start_time = current_unix_timestamp_ms().unwrap_or(0);

    let mut packets_captured = 0;

    // Start packet capture
    let mut batch = Vec::new();
    let mut video_pid: Option<u16> = Some(0xFFFF);
    let mut video_codec: Option<Codec> = Some(Codec::NONE);
    let mut current_video_frame = Vec::<StreamData>::new();
    let mut pmt_info: PmtInfo = PmtInfo {
        pid: 0xFFFF,
        packet: Vec::new(),
    };

    while let Some(packet) = prx.recv().await {
        if packet_count > 0 && packets_captured > packet_count {
            running.store(false, Ordering::SeqCst);
            break;
        }
        packets_captured += 1;

        if !no_progress {
            print!(".");
            // flush stdout (wastes resources) TODO: output in ncurses or something w/less output
            //std::io::stdout().flush().unwrap();
        }

        // Check if chunk is MPEG-TS or SMPTE 2110
        let chunk_type = is_mpegts_or_smpte2110(&packet[payload_offset..]);
        if chunk_type != 1 {
            if chunk_type == 0 {
                error!("Not MPEG-TS or SMPTE 2110");
                hexdump(&packet, 0, packet.len());
            }
            is_mpegts = false;
        }

        let chunks = if is_mpegts {
            process_mpegts_packet(payload_offset, &packet, packet_size, start_time)
        } else {
            process_smpte2110_packet(payload_offset, &packet, packet.len(), start_time)
        };

        // Process each chunk
        for mut stream_data in chunks {
            if debug_on {
                hexdump(
                    &stream_data.packet,
                    stream_data.packet_start,
                    stream_data.packet_len,
                );
            }

            // Extract the necessary slice for PID extraction and parsing
            let packet_chunk = &stream_data.packet
                [stream_data.packet_start..stream_data.packet_start + stream_data.packet_len];

            let mut pid: u16 = 0xFFFF;

            if is_mpegts {
                pid = stream_data.pid;
                // Handle PAT and PMT packets
                match pid {
                    PAT_PID => {
                        debug!("ProcessPacket: PAT packet detected with PID {}", pid);
                        pmt_info = parse_and_store_pat(&packet_chunk);
                        // Print TR 101 290 errors
                        if show_tr101290 {
                            info!("STATUS::TR101290:ERRORS: {}", tr101290_errors);
                        }
                    }
                    _ => {
                        // Check if this is a PMT packet
                        if pid == pmt_info.pid {
                            debug!("ProcessPacket: PMT packet detected with PID {}", pid);
                            // Update PID_MAP with new stream types
                            update_pid_map(&packet_chunk, &pmt_info.packet);
                            // Identify the video PID (if not already identified)
                            if let Some((new_pid, new_codec)) = identify_video_pid(&packet_chunk) {
                                if video_pid.map_or(true, |vp| vp != new_pid) {
                                    video_pid = Some(new_pid);
                                    info!(
                                        "STATUS::VIDEO_PID:CHANGE: to {}/{} from {}/{}",
                                        new_pid,
                                        new_codec.clone(),
                                        video_pid.unwrap(),
                                        video_codec.unwrap()
                                    );
                                    video_codec = Some(new_codec.clone());
                                    // Reset video frame as the video stream has changed
                                    current_video_frame.clear();
                                } else if video_codec != Some(new_codec.clone()) {
                                    info!(
                                        "STATUS::VIDEO_CODEC:CHANGE: to {} from {}",
                                        new_codec,
                                        video_codec.unwrap()
                                    );
                                    video_codec = Some(new_codec);
                                    // Reset video frame as the codec has changed
                                    current_video_frame.clear();
                                }
                            }
                        }
                    }
                }
            }

            // Check for TR 101 290 errors
            process_packet(
                &mut stream_data,
                &mut tr101290_errors,
                is_mpegts,
                pmt_info.pid,
            );

            if pid == 0x1FFF && is_mpegts {
                // clear the Arc so it can be reused
                stream_data.packet = Arc::new(Vec::new()); // Create a new Arc<Vec<u8>> for the next packet
                                                           // Skip null packets
                continue;
            }
            batch.push(stream_data);

            //info!("STATUS::PACKETS:CAPTURED: {}", packets_captured);
            // Check if batch is full
            if !no_zmq {
                if batch.len() >= batch_size {
                    //info!("STATUS::BATCH:SEND: {}", batch.len());
                    // Send the batch to the channel
                    tx.send(batch).await.unwrap();
                    // release the packet Arc so it can be reused
                    batch = Vec::new(); // Create a new Vec for the next batch
                } else {
                    //info!("STATUS::BATCH:WAIT: {}", batch.len());
                }
            } else {
                // go through each stream_data and release the packet Arc so it can be reused
                for stream_data in batch.iter_mut() {
                    stream_data.packet = Arc::new(Vec::new()); // Create a new Arc<Vec<u8>> for the next packet
                }
                // disgard the batch, we don't send it anywhere
                batch.clear();
                batch = Vec::new(); // Create a new Vec for the next batch
                                    //info!("STATUS::BATCH:DISCARD: {}", batch.len());
            }
        }
    }
    /*Err(e) => {
    error!("Error capturing packet: {:?}", e);
    }*/

    println!("Exiting rscap probe");

    // Send stop signal
    tx.send(Vec::new()).await.unwrap();
    drop(tx);

    // Wait for the zmq_thread to finish
    zmq_thread.await.unwrap();
    capture_task.await.unwrap();
}

// Check if the packet is MPEG-TS or SMPTE 2110
fn is_mpegts_or_smpte2110(packet: &[u8]) -> i32 {
    // Check for MPEG-TS (starts with 0x47 sync byte)
    if packet.starts_with(&[0x47]) {
        return 1;
    }

    // Basic check for RTP (which SMPTE ST 2110 uses)
    // This checks if the first byte is 0x80 or 0x81
    // This might need more robust checks based on requirements
    if packet.len() > 12 && (packet[0] == 0x80 || packet[0] == 0x81) {
        // TODO: Check payload type or other RTP header fields here if necessary
        return 2; // Assuming it's SMPTE ST 2110 for now
    }

    0 // Not MPEG-TS or SMPTE 2110
}

// ## RFC 4175 SMPTE2110 header functions ##
/*const RFC_4175_EXT_SEQ_NUM_LEN: usize = 2;
const RFC_4175_HEADER_LEN: usize = 6; // Note: extended sequence number not included*/ // TODO: implement RFC 4175 SMPTE2110 header functions

fn get_extended_sequence_number(buf: &[u8]) -> u16 {
    ((buf[0] as u16) << 8) | buf[1] as u16
}

fn get_line_length(buf: &[u8]) -> u16 {
    ((buf[0] as u16) << 8) | buf[1] as u16
}

fn get_line_field_id(buf: &[u8]) -> u8 {
    buf[2] >> 7
}

fn get_line_number(buf: &[u8]) -> u16 {
    ((buf[2] as u16 & 0x7f) << 8) | buf[3] as u16
}

fn get_line_continuation(buf: &[u8]) -> u8 {
    buf[4] >> 7
}

fn get_line_offset(buf: &[u8]) -> u16 {
    ((buf[4] as u16 & 0x7f) << 8) | buf[5] as u16
}
// ## End of RFC 4175 SMPTE2110 header functions ##

// Process the packet and return a vector of SMPTE ST 2110 packets
fn process_smpte2110_packet(
    payload_offset: usize,
    packet: &Arc<Vec<u8>>,
    packet_size: usize,
    start_time: u64,
) -> Vec<StreamData> {
    let mut streams = Vec::new();

    // Check if the packet is large enough to contain an RTP header
    if packet_size > payload_offset + 12 {
        // Check for RTP header marker
        if packet[payload_offset] == 0x80 || packet[payload_offset] == 0x81 {
            let rtp_packet = &packet[payload_offset..];

            // Create an RtpReader
            if let Ok(rtp) = RtpReader::new(rtp_packet) {
                // Extract the timestamp and payload type
                let timestamp = rtp.timestamp();
                let payload_type = rtp.payload_type();

                // Calculate the actual start of the RTP payload
                let rtp_payload_offset = payload_offset + rtp.payload_offset();

                // Calculate the length of the RTP payload
                let rtp_payload_length = packet_size - rtp_payload_offset;

                // Extract SMPTE 2110 specific fields
                let line_length = get_line_length(rtp_packet);
                let line_number = get_line_number(rtp_packet);
                let extended_sequence_number = get_extended_sequence_number(rtp_packet);
                let line_offset = get_line_offset(rtp_packet);
                let field_id = get_line_field_id(rtp_packet);

                let line_continuation = get_line_continuation(rtp_packet);

                // Use payload type as PID (for the purpose of this example)
                let pid = payload_type as u16;
                let stream_type = payload_type.to_string();

                // Create new StreamData instance
                let mut stream_data = StreamData::new(
                    Arc::clone(packet),
                    rtp_payload_offset,
                    rtp_payload_length,
                    pid,
                    stream_type,
                    start_time,
                    timestamp as u64,
                    0,
                );

                // Update StreamData stats and RTP fields
                stream_data
                    .update_stats(rtp_payload_length, current_unix_timestamp_ms().unwrap_or(0));
                stream_data.set_rtp_fields(
                    timestamp,
                    payload_type,
                    payload_type.to_string(),
                    line_number,
                    line_offset,
                    line_length,
                    field_id,
                    line_continuation,
                    extended_sequence_number,
                );

                // Add the StreamData to the stream list
                streams.push(stream_data);
            } else {
                hexdump(&packet, 0, packet_size);
                error!("Error parsing RTP header, not SMPTE ST 2110");
            }
        } else {
            hexdump(&packet, 0, packet_size);
            error!("No RTP header detected, not SMPTE ST 2110");
        }
    } else {
        hexdump(&packet, 0, packet_size);
        error!("Packet too small, not SMPTE ST 2110");
    }

    streams
}

// Process the packet and return a vector of MPEG-TS packets
fn process_mpegts_packet(
    payload_offset: usize,
    packet: &Arc<Vec<u8>>,
    packet_size: usize,
    start_time: u64,
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

            let stream_type = determine_stream_type(pid); // Implement this function based on PAT/PMT parsing
            let timestamp = ((chunk[4] as u64) << 25)
                | ((chunk[5] as u64) << 17)
                | ((chunk[6] as u64) << 9)
                | ((chunk[7] as u64) << 1)
                | ((chunk[8] as u64) >> 7);
            let continuity_counter = chunk[3] & 0x0F;

            let mut stream_data = StreamData::new(
                Arc::clone(packet),
                start,
                packet_size,
                pid,
                stream_type,
                start_time,
                timestamp,
                continuity_counter,
            );
            stream_data.update_stats(packet_size, current_unix_timestamp_ms().unwrap_or(0));
            streams.push(stream_data);
        } else {
            error!("ProcessPacket: Not MPEG-TS");
            hexdump(&packet, start, packet_size);
            read_size = 1; // Skip to the next byte
        }
        start += read_size;
    }

    streams
}

// Print a hexdump of the packet
fn hexdump(packet_arc: &Arc<Vec<u8>>, packet_offset: usize, packet_len: usize) {
    let packet = &packet_arc[packet_offset..packet_offset + packet_len];
    let pid = extract_pid(packet);
    println!("--------------------------------------------------");
    // print in rows of 16 bytes
    println!("PacketDump: PID {} Packet length: {}", pid, packet_len);
    let mut packet_dump = String::new();
    for (i, chunk) in packet.iter().take(packet_len).enumerate() {
        if i % 16 == 0 {
            packet_dump.push_str(&format!("\n{:04x}: ", i));
        }
        packet_dump.push_str(&format!("{:02x} ", chunk));
    }
    println!("{}", packet_dump);
    println!("");
    println!("--------------------------------------------------");
}
