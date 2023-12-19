/*
 * rscap: probe - Rust Stream Capture with pcap, output to ZeroMQ
 *
 * Written in 2023 by Chris Kennedy (C) LTN Global
 *
 * License: LGPL v2.1
 *
 */

extern crate rtp_rs as rtp;
extern crate zmq;
use clap::Parser;
use lazy_static::lazy_static;
use log::{debug, error, info};
use pcap::Capture;
use rtp::RtpReader;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::fmt;
use std::io::Write;
use std::net::{Ipv4Addr, UdpSocket};
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use firestorm::{profile_fn, profile_method, profile_section};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};

// constant for PAT PID
const PAT_PID: u16 = 0;

// global variable to store last PAT packet
static mut LAST_PAT_PACKET: Option<Vec<u8>> = None;

// global variable to store PMT PID (initially set to an invalid PID)
static mut PMT_PID: u16 = 0xFFFF;

// global variable to store the MpegTS PID Map (initially empty)
lazy_static! {
    static ref PID_MAP: Mutex<HashMap<u16, StreamData>> = Mutex::new(HashMap::new());
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

// StreamData struct
#[derive(Clone)]
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
    data: Vec<u8>,   // The actual MPEG-TS packet data
    pmt_data: Vec<u8>,
    // SMPTE 2110 fields
    rtp_timestamp: u32,
    rtp_payload_type: u8,
    rtp_payload_type_name: String,
    rtp_line_number: u16,
    rtp_line_offset: u16,
    rtp_line_length: u16,
    rtp_field_id: u8,
}

// StreamData implementation
impl StreamData {
    fn new(
        packet: &[u8],
        pid: u16,
        stream_type: String,
        start_time: u64,
        timestamp: u64,
        continuity_counter: u8,
    ) -> Self {
        let bitrate = 0;
        let bitrate_max = 0;
        let bitrate_min = 0;
        let bitrate_avg = 0;
        let iat = 0;
        let iat_max = 0;
        let iat_min = 0;
        let iat_avg = 0;
        let error_count = 0;
        let last_arrival_time = current_unix_timestamp_ms().unwrap_or(0);
        StreamData {
            pid,
            pmt_pid: 0xFFFF,
            program_number: 0,
            stream_type,
            continuity_counter,
            timestamp,
            bitrate,
            bitrate_max,
            bitrate_min,
            bitrate_avg,
            iat,
            iat_max,
            iat_min,
            iat_avg,
            error_count,
            last_arrival_time,
            start_time,    // Initialize start time
            total_bits: 0, // Initialize total bits
            count: 0,      // Initialize count
            data: packet.to_vec(),
            pmt_data: Vec::new(),
            // SMPTE 2110 fields
            rtp_timestamp: 0,
            rtp_payload_type: 0,
            rtp_payload_type_name: "".to_string(),
            rtp_line_number: 0,
            rtp_line_offset: 0,
            rtp_line_length: 0,
            rtp_field_id: 0,
        }
    }
    fn set_rtp_fields(
        &mut self,
        rtp_timestamp: u32,
        rtp_payload_type: u8,
        rtp_payload_type_name: String,
        rtp_line_number: u16,
        rtp_line_offset: u16,
        rtp_line_length: u16,
        rtp_field_id: u8,
    ) {
        self.rtp_timestamp = rtp_timestamp;
        self.rtp_payload_type = rtp_payload_type;
        self.rtp_payload_type_name = rtp_payload_type_name;
        self.rtp_line_number = rtp_line_number;
        self.rtp_line_offset = rtp_line_offset;
        self.rtp_line_length = rtp_line_length;
        self.rtp_field_id = rtp_field_id;
    }
    fn set_pmt_pid(&mut self, pmt_pid: u16) {
        profile_fn!(set_pmt_pid);
        self.pmt_pid = pmt_pid;
    }
    fn add_pmt_data(&mut self, pmt_data: &[u8]) {
        profile_fn!(add_pmt_data);
        self.pmt_data = pmt_data.to_vec();
    }
    fn update_program_number(&mut self, program_number: u16) {
        profile_fn!(update_program_number);
        self.program_number = program_number;
    }
    fn update_stream_type(&mut self, stream_type: String) {
        profile_fn!(update_stream_type);
        self.stream_type = stream_type;
    }
    fn update_timestamp(&mut self, timestamp: u64) {
        profile_fn!(update_timestamp);
        self.timestamp = timestamp;
    }
    fn increment_error_count(&mut self, error_count: u32) {
        profile_fn!(increment_error_count);
        self.error_count += error_count;
    }
    fn increment_count(&mut self, count: u32) {
        profile_fn!(increment_count);
        self.count += count;
    }
    fn set_continuity_counter(&mut self, continuity_counter: u8) {
        profile_fn!(set_continuity_counter);
        // check for continuity continuous increment and wrap around from 0 to 15
        let previous_continuity_counter = self.continuity_counter;
        self.continuity_counter = continuity_counter & 0x0F;
        // check if we incremented without loss
        if self.continuity_counter != previous_continuity_counter + 1 {
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
        profile_fn!(update_stats);
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
    profile_fn!(tr101290_p1_check);
    // p1
    if packet[0] != 0x47 {
        errors.sync_byte_errors += 1;
    }

    // TODO: ... other checks, updating the respective counters ...
}

// TR 101 290 Priority 2 Check
fn tr101290_p2_check(packet: &[u8], errors: &mut Tr101290Errors) {
    profile_fn!(tr101290_p2_check);
    // p2

    if (packet[1] & 0x80) != 0 {
        errors.transport_error_indicator_errors += 1;
    }
    // TODO: ... other checks, updating the respective counters ...
}

// Invoke this function for each MPEG-TS packet
fn process_packet(stream_data_packet: &StreamData, errors: &mut Tr101290Errors, is_mpegts: bool) {
    profile_fn!(process_packet);
    let packet: &[u8] = &stream_data_packet.data;
    tr101290_p1_check(packet, errors);
    tr101290_p2_check(packet, errors);

    let pid = extract_pid(packet);
    let arrival_time = current_unix_timestamp_ms().unwrap_or(0);

    let mut pid_map = PID_MAP.lock().unwrap();
    let mut found_pid = false;

    // Check if the PID map already has an entry for this PID
    match pid_map.get_mut(&pid) {
        Some(stream_data) => {
            // Existing StreamData instance found, update it
            stream_data.update_stats(packet.len(), arrival_time);
            stream_data.increment_count(1);
            if stream_data.pid != 0x1FFF && is_mpegts {
                stream_data.set_continuity_counter(stream_data_packet.continuity_counter);
            }
            let uptime = arrival_time - stream_data.start_time;

            // create json object of stats
            let json_stats = json!({
                "type": "mpegts_stats",
                "pid": stream_data.pid,
                "stream_type": stream_data.stream_type,
                "bitrate": stream_data.bitrate,
                "bitrate_max": stream_data.bitrate_max,
                "bitrate_min": stream_data.bitrate_min,
                "bitrate_avg": stream_data.bitrate_avg,
                "iat": stream_data_packet.iat,
                "iat_max": stream_data.iat_max,
                "iat_min": stream_data.iat_min,
                "iat_avg": stream_data.iat_avg,
                "errors": stream_data.error_count,
                "continuity_counter": stream_data_packet.continuity_counter,
                "timestamp": stream_data_packet.timestamp,
                "uptime": uptime,
            });

            // log json stats
            info!("STATUS::PACKET:MODIFY[{}] {}", stream_data.pid, json_stats);
            found_pid = true;
        }
        None => {
            // No StreamData instance found for this PID, possibly no PMT yet
            let mut pmt_pid = 0xFFFF;
            unsafe {
                pmt_pid = PMT_PID;
            }

            if pmt_pid != 0xFFFF {
                info!("ProcessPacket: New PID {} Found, adding to PID map.", pid);
            } else {
                // PMT packet not found yet, add the stream_data_packet to the pid_map
                let mut stream_data_clone = stream_data_packet.clone();
                stream_data_clone.update_stats(packet.len(), arrival_time);
                pid_map.insert(pid, stream_data_clone.clone());
                // create json object of stats
                let json_stats = json!({
                    "type": "mpegts_stats",
                    "pid": stream_data_clone.pid,
                    "stream_type": stream_data_clone.stream_type,
                    "bitrate": stream_data_clone.bitrate,
                    "bitrate_max": stream_data_clone.bitrate_max,
                    "bitrate_min": stream_data_clone.bitrate_min,
                    "bitrate_avg": stream_data_clone.bitrate_avg,
                    "iat": stream_data_packet.iat,
                    "iat_max": stream_data_clone.iat_max,
                    "iat_min": stream_data_clone.iat_min,
                    "iat_avg": stream_data_clone.iat_avg,
                    "errors": stream_data_clone.error_count,
                    "continuity_counter": stream_data_packet.continuity_counter,
                    "timestamp": stream_data_packet.timestamp,
                    "uptime": 0,
                });
                info!(
                    "STATUS::PACKET:ADD[{}] {}",
                    stream_data_clone.pid, json_stats
                );
            }
        }
    }

    // PID not found, add the stream_data_packet to the pid_map, probably before we have seen the PMT
    if !found_pid {
        // PID not found, add the stream_data_packet to the pid_map
        pid_map.insert(pid, stream_data_packet.clone());
    }
}

// Function to get the current Unix timestamp in milliseconds
fn current_unix_timestamp_ms() -> Result<u64, String> {
    profile_fn!(current_unix_timestamp_ms);
    let now = SystemTime::now();
    match now.duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            let milliseconds = duration.as_secs() * 1000 + u64::from(duration.subsec_millis());
            Ok(milliseconds)
        }
        Err(e) => Err(format!("System time is before the UNIX epoch: {}", e)),
    }
}

// Implement a function to extract PID from a packet
fn extract_pid(packet: &[u8]) -> u16 {
    profile_fn!(extract_pid);
    // Extract PID from packet
    // (You'll need to adjust the indices according to your packet format)
    ((packet[1] as u16 & 0x1F) << 8) | packet[2] as u16
}

// Helper function to parse PAT and update global PAT packet storage
fn parse_and_store_pat(packet: &[u8]) {
    profile_fn!(parse_and_store_pat);
    let pat_entries = parse_pat(packet);
    unsafe {
        // Store the specific PAT chunk for later use
        LAST_PAT_PACKET = Some(packet.to_vec());
    }

    // Assuming there's only one program for simplicity, update PMT PID
    if let Some(first_entry) = pat_entries.first() {
        unsafe { PMT_PID = first_entry.pmt_pid };
    }
}

fn parse_pat(packet: &[u8]) -> Vec<PatEntry> {
    profile_fn!(parse_pat);
    let mut entries = Vec::new();

    // Check if Payload Unit Start Indicator (PUSI) is set
    let pusi = (packet[1] & 0x40) != 0;
    let adaptation_field_control = (packet[3] & 0x30) >> 4;
    let mut pat_start = 4; // start after TS header

    if adaptation_field_control == 0x02 || adaptation_field_control == 0x03 {
        let adaptation_field_length = packet[4] as usize;
        pat_start += 1 + adaptation_field_length; // +1 for the length byte itself
        debug!(
            "ParsePAT: Adaptation Field Length: {}",
            adaptation_field_length
        );
    } else {
        debug!(
            "ParsePAT: Skipping Adaptation Field Control: {}",
            adaptation_field_control
        );
    }

    if pusi {
        // PUSI is set, so the first byte after the TS header is the pointer field
        let pointer_field = packet[pat_start] as usize;
        //pat_start += 1 + pointer_field; // Skip pointer field
        debug!(
            "ParsePAT: PUSI set as {}, skipping pointer field {} as packet value {} {}",
            pusi, pointer_field, packet[0], packet[1]
        );
    } else {
        debug!("ParsePAT: PUSI not set as {}", pusi);
    }

    debug!("ParsePAT: PAT start: {}", pat_start);

    // Check for the presence of a pointer field
    let pointer_field = packet[pat_start] as usize;
    pat_start += 1 + pointer_field; // Move past the pointer field

    let mut i = pat_start; // Starting index of the PAT data

    while i + 4 <= packet.len() {
        let program_number = ((packet[i] as u16) << 8) | (packet[i + 1] as u16);
        // Mask the lower 13 bits for the PMT PID
        let pmt_pid = (((packet[i + 2] as u16) & 0x1F) << 8) | (packet[i + 3] as u16);

        //debug!("ParsePAT: Packet1 {}, Packet2 {}, Packet3 {}, Packet4 {}", packet[i], packet[i + 1], packet[i + 2], packet[i + 3]);

        if program_number != 0 && program_number != 65535 && pmt_pid != 0 && program_number < 30
        /* FIXME: kludge fix for now */
        {
            debug!(
                "ParsePAT: Program Number: {} PMT PID: {}",
                program_number, pmt_pid
            );
            entries.push(PatEntry {
                program_number,
                pmt_pid,
            });
        }

        i += 4;
    }

    entries
}

fn parse_pmt(packet: &[u8], pmt_pid: u16) -> Pmt {
    profile_fn!(parse_pmt);
    let mut entries = Vec::new();
    let program_number = ((packet[8] as u16) << 8) | (packet[9] as u16);

    // Calculate the starting position for stream entries
    let section_length = (((packet[6] as usize) & 0x0F) << 8) | packet[7] as usize;
    let program_info_length = (((packet[15] as usize) & 0x0F) << 8) | packet[16] as usize;
    let mut i = 17 + program_info_length; // Starting index of the first stream in the PMT

    debug!(
        "ParsePMT: Program Number: {} PMT PID: {} starting at position {}",
        program_number, pmt_pid, i
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

// Modify the function to use the stored PAT packet
fn update_pid_map(pmt_packet: &[u8]) {
    profile_fn!(update_pid_map);
    let mut pid_map = PID_MAP.lock().unwrap();

    // Process the stored PAT packet to find program numbers and corresponding PMT PIDs
    let program_pids = unsafe {
        LAST_PAT_PACKET
            .as_ref()
            .map_or_else(Vec::new, |pat_packet| parse_pat(pat_packet))
    };

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
            let pmt = parse_pmt(pmt_packet, pmt_pid);

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
                    let mut stream_data = StreamData::new(
                        &[],
                        stream_pid,
                        stream_type.to_string(),
                        timestamp,
                        timestamp,
                        0,
                    );
                    // update stream_data stats
                    stream_data.update_stats(pmt_packet.len(), timestamp);

                    // create json object of stats
                    let json_stats = json!({
                        "type": "mpegts_stats",
                        "pid": stream_data.pid,
                        "stream_type": stream_data.stream_type,
                        "bitrate": stream_data.bitrate,
                        "bitrate_max": stream_data.bitrate_max,
                        "bitrate_min": stream_data.bitrate_min,
                        "bitrate_avg": stream_data.bitrate_avg,
                        "iat": stream_data.iat,
                        "iat_max": stream_data.iat_max,
                        "iat_min": stream_data.iat_min,
                        "iat_avg": stream_data.iat_avg,
                        "errors": stream_data.error_count,
                        "continuity_counter": stream_data.continuity_counter,
                        "timestamp": stream_data.timestamp,
                        "uptime": 0,
                    });

                    // log json stats
                    info!("STATUS::STREAM:CREATE[{}] {}", stream_data.pid, json_stats);

                    pid_map.insert(stream_pid, stream_data);
                } else {
                    // get the stream data so we can update it
                    let stream_data = pid_map.get_mut(&stream_pid).unwrap();

                    // update the stream type
                    stream_data.update_stream_type(stream_type.to_string());
                    // update the count

                    // create json object of stats
                    let json_stats = json!({
                        "type": "mpegts_stats",
                        "pid": stream_data.pid,
                        "stream_type": stream_data.stream_type,
                        "bitrate": stream_data.bitrate,
                        "bitrate_max": stream_data.bitrate_max,
                        "bitrate_min": stream_data.bitrate_min,
                        "bitrate_avg": stream_data.bitrate_avg,
                        "iat": stream_data.iat,
                        "iat_max": stream_data.iat_max,
                        "iat_min": stream_data.iat_min,
                        "iat_avg": stream_data.iat_avg,
                        "errors": stream_data.error_count,
                        "continuity_counter": stream_data.continuity_counter,
                        "timestamp": stream_data.timestamp,
                        "uptime": 0,
                    });

                    // log json stats
                    debug!("STATUS::STREAM:UPDATE[{}] {}", stream_data.pid, json_stats);
                }
            }
        } else {
            error!("UpdatePIDmap: Skipping PMT PID: {} as it does not match with current PMT packet PID", pmt_pid);
        }
    }
}

fn determine_stream_type(pid: u16) -> String {
    profile_fn!(determine_stream_type);
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

/// RScap Probe Configuration
#[derive(Parser, Debug)]
#[clap(
    author = "Chris Kennedy",
    version = "1.0",
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
    #[clap(long, env = "READ_TIME_OUT", default_value_t = 60000)]
    read_time_out: i32,

    /// Sets the target port
    #[clap(long, env = "TARGET_PORT", default_value_t = 5556)]
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
    #[clap(long, env = "SOURCE_PORT", default_value_t = 10000)]
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

    /// number of packets to capture
    #[clap(long, env = "PACKET_COUNT", default_value_t = 0)]
    packet_count: u64,
}

fn main() {
    if firestorm::enabled() {
        if let Err(e) = firestorm::bench("./flames/", || {
            rscap();
        }) {
            println!("Error occurred: {:?}", e);
        }
    } else {
        rscap();
    }
}

// MAIN Function
fn rscap() {
    println!("Starting rscap probe");
   
    dotenv::dotenv().ok(); // read .env file

    let source_device_ip: &str = "0.0.0.0";

    let args = Args::parse();

    // Use the parsed arguments directly
    let mut batch_size = args.batch_size;
    let payload_offset = args.payload_offset;
    let packet_size = args.packet_size;
    let read_time_out = args.read_time_out;
    let target_port = args.target_port;
    let target_ip = args.target_ip;
    let source_device = args.source_device;
    let source_ip = args.source_ip;
    let source_protocol = args.source_protocol;
    let source_port = args.source_port;
    let debug_on = args.debug_on;
    let silent = args.silent;
    #[cfg(not(target_os = "linux"))]
    let use_wireless = args.use_wireless;
    let send_json_header = args.send_json_header;
    let packet_count = args.packet_count;

    if silent {
        // set log level to error
        std::env::set_var("RUST_LOG", "error");
    }

    // calculate read size based on batch size and packet size
    let read_size: i32 = (packet_size as i32 * batch_size as i32) + payload_offset as i32; // pcap read size

    let mut is_mpegts = true; // Default to true, update based on actual packet type

    // Initialize logging
    let _ = env_logger::try_init();

    // device ip address
    let mut interface_addr = source_device_ip.parse::<Ipv4Addr>().expect(&format!(
        "Invalid IP address format in source_device_ip {}",
        source_device_ip
    ));

    // Get the selected device's details
    let mut target_device_found = false;
    let devices = pcap::Device::list().unwrap();
    // List all devices and their flags
    for device in &devices {
        info!("Device: {:?}, Flags: {:?}", device.name, device.flags);
    }
    #[cfg(target_os = "linux")]
    let mut target_device = devices
        .clone()
        .into_iter()
        .find(|d| d.name != "lo") // Exclude loopback device
        .expect("No valid devices found");

    #[cfg(not(target_os = "linux"))]
    let mut target_device = devices
        .clone()
        .into_iter()
        .find(|d| {
            d.flags.is_up()
                && !d.flags.is_loopback()
                && d.flags.is_running()
                && (!d.flags.is_wireless() || use_wireless)
        })
        .expect(&format!("No valid devices found {}", devices.len()));

    info!("Default device: {:?}", target_device.name);

    // If source_device is auto, find the first valid device
    if source_device == "auto" || source_device == "" {
        info!("Auto-selecting device...");

        // Find the first valid device
        for device in pcap::Device::list().unwrap() {
            debug!("Device {:?}", device);

            // check flags for device up
            #[cfg(not(target_os = "linux"))]
            if !device.flags.is_up() {
                continue;
            }
            // check if device is loopback
            #[cfg(not(target_os = "linux"))]
            if device.flags.is_loopback() {
                continue;
            }
            // check if device is ethernet
            #[cfg(not(target_os = "linux"))]
            if device.flags.is_wireless() {
                if !use_wireless {
                    continue;
                }
            }
            // check if device is running
            #[cfg(not(target_os = "linux"))]
            if !device.flags.is_running() {
                continue;
            }

            // check if device has an IPv4 address
            for addr in device.addresses.iter() {
                if let std::net::IpAddr::V4(ipv4_addr) = addr.addr {
                    // check if loopback
                    if ipv4_addr.is_loopback() {
                        continue;
                    }
                    target_device_found = true;

                    // Found through auto-detection, set interface_addr
                    info!(
                        "Found IPv4 target device {} with ip {}",
                        source_device, ipv4_addr
                    );
                    interface_addr = ipv4_addr;
                    target_device = device;
                    break;
                }
            }
            // break out of loop if target device is found
            if target_device_found {
                break;
            }
        }
    } else {
        // Use the specified device instead of auto-detection
        info!("Using specified device {}", source_device);

        // Find the specified device
        #[cfg(not(target_os = "linux"))]
        let target_device_discovered = devices
            .into_iter()
            .find(|d| {
                d.name == source_device
                    && d.flags.is_up()
                    && d.flags.is_running()
                    && (!d.flags.is_wireless() || use_wireless)
            })
            .expect(&format!("Target device not found {}", source_device));

        #[cfg(target_os = "linux")]
        let target_device_discovered = devices
            .into_iter()
            .find(|d| d.name == source_device)
            .expect(&format!("Target device not found {}", source_device));

        // Check if device has an IPv4 address
        info!("Target Device: {:?}", target_device_discovered);
        for addr in target_device_discovered.addresses.iter() {
            if let std::net::IpAddr::V4(ipv4_addr) = addr.addr {
                info!("Found ipv4_addr: {:?}", ipv4_addr);
                interface_addr = ipv4_addr;
                target_device_found = true;
                target_device = target_device_discovered;
                break;
            }
        }
    }

    // Device not found
    if !target_device_found {
        error!("Target device {} not found", source_device);
        return;
    }

    // Join multicast group
    let multicast_addr = source_ip.parse::<Ipv4Addr>().expect(&format!(
        "Invalid IP address format in source_ip {}",
        source_ip
    ));

    let socket = UdpSocket::bind("0.0.0.0:0").expect("Failed to bind socket");
    socket
        .join_multicast_v4(&multicast_addr, &interface_addr)
        .expect(&format!(
            "Failed to join multicast group on interface {}",
            source_device
        ));

    #[cfg(not(target_os = "linux"))]
    let promiscuous: bool = false;

    #[cfg(target_os = "linux")]
    let promiscuous: bool = true;

    // Filter pcap
    let source_host_and_port = format!(
        "{} dst port {} and ip dst host {}",
        source_protocol, source_port, source_ip
    );

    let (ptx, prx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel();

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    let capture_thread = thread::spawn(move || {
        let mut cap = Capture::from_device(target_device).unwrap()
            .promisc(promiscuous)
            .timeout(read_time_out)
            .snaplen(read_size)
            .open().unwrap();

        cap.filter(&source_host_and_port, true).unwrap();

        while running_clone.load(Ordering::SeqCst) {
            match cap.next_packet() {
                Ok(packet) => {
                    // Send the packet data directly without copying
                    ptx.send(packet.data.to_owned()).unwrap();
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
            if debug_on {
                let stats = cap.stats().unwrap();
                println!("Current stats: Received: {}, Dropped: {}, Interface Dropped: {}",
                         stats.received, stats.dropped, stats.if_dropped);
            }            
        }

        let stats = cap.stats().unwrap();
        println!("Packet capture statistics:");
        println!("Received: {}", stats.received);
        println!("Dropped: {}", stats.dropped);
        println!("Interface Dropped: {}", stats.if_dropped);
    });

    // Setup channel for passing data between threads
    let (tx, rx) = mpsc::channel::<Vec<StreamData>>();

    // Spawn a new thread for ZeroMQ communication
    let zmq_thread = thread::spawn(move || {
        // Setup ZeroMQ publisher
        let context = zmq::Context::new();
        let publisher = context.socket(zmq::PUB).unwrap();
        let source_port_ip = format!("tcp://{}:{}", target_ip, target_port);
        publisher.bind(&source_port_ip).unwrap();

        let mut total_bytes = 0;
        let mut count = 0;
        for batch in rx {
            // Check for a stop signal
            if batch.is_empty() {
                break; // Exit the loop if a stop signal is received
            }
            // ... ZeroMQ sending logic ...
            //let batched_data = batch.concat();
            for stream_data in batch.iter() {
                // Send chunk of data as multipart message
                let chunk_size = stream_data.data.len();
                total_bytes += chunk_size;
                count += 1;

                let mut format_str = "unknown";
                let format_index = is_mpegts_or_smpte2110(&stream_data.data);
                if format_index == 1 {
                    format_str = "mpegts";
                } else if format_index == 2 {
                    format_str = "smpte2110";
                }
                // Construct JSON header for batched data
                let json_header = json!({
                    "type": "mpegts_chunk",
                    "content_length": chunk_size,
                    "total_bytes": total_bytes,
                    "count": count,
                    "source_ip": source_ip,
                    "source_port": source_port,
                    "source_device": source_device,
                    "target_ip": target_ip,
                    "target_port": target_port,
                    "format": format_str,
                    "count": count,
                    "timestamp": current_unix_timestamp_ms().unwrap_or(0),
                    "chunk_size": chunk_size,
                    "batch_size": batch_size,
                });

                // Check if JSON header is enabled
                if send_json_header {
                    // Send JSON header as multipart message
                    publisher
                        .send(json_header.to_string().as_bytes(), zmq::SNDMORE)
                        .unwrap();
                }

                publisher.send(stream_data.data.clone(), 0).unwrap();

                // Print progress
                log::info!("STATUS::ZEROMQ:TX {}", json_header);
            }
        }
    });

    // Perform TR 101 290 checks
    let mut tr101290_errors = Tr101290Errors::new();

    // start time
    let start_time = current_unix_timestamp_ms().unwrap_or(0);

    let mut packets_captured = 0;

    // Start packet capture
    let mut batch = Vec::new();
    loop {
        if packet_count > 0 && packets_captured > packet_count {
            running.store(false, Ordering::SeqCst);
            break;
        }
        match prx.recv() {
            Ok(packet) => {
                packets_captured += 1;
                
                if silent {
                    print!(".");
                    // flush stdout
                    std::io::stdout().flush().unwrap();
                }

                // Check if chunk is MPEG-TS or SMPTE 2110
                let chunk_type = is_mpegts_or_smpte2110(&packet[payload_offset..]);
                if chunk_type != 1 {
                    if chunk_type == 0 {
                        error!("Not MPEG-TS or SMPTE 2110");
                        hexdump(&packet);
                    }
                    is_mpegts = false;
                }

                let chunks = if is_mpegts {
                    process_mpegts_packet(payload_offset, &packet, packet_size, start_time)
                } else {
                    process_smpte2110_packet(payload_offset, &packet, packet_size, start_time)
                };

                if !is_mpegts {
                    batch_size = 1; // Set batch size to 1 for SMPTE 2110
                }

                // Process each chunk
                for stream_data in chunks {
                    if debug_on {
                        hexdump(&stream_data.data); // Use stream_data.data to access the raw packet data
                    }

                    // Handle PAT and PMT packets
                    let pid = extract_pid(&stream_data.data);
                    match pid {
                        PAT_PID => {
                            log::debug!("ProcessPacket: PAT packet detected with PID {}", pid);
                            parse_and_store_pat(&stream_data.data);
                        }
                        _ => {
                            // Check if this is a PMT packet
                            unsafe {
                                if pid == PMT_PID {
                                    log::debug!(
                                        "ProcessPacket: PMT packet detected with PID {}",
                                        pid
                                    );
                                    // Update PID_MAP with new stream types
                                    update_pid_map(&stream_data.data);
                                }
                            }
                        }
                    }

                    // Check for TR 101 290 errors
                    process_packet(&stream_data, &mut tr101290_errors, is_mpegts);

                    // Print TR 101 290 errors
                    match serde_json::to_string(&tr101290_errors) {
                        Ok(json_string) => info!("STATUS::TR101290:ERRORS: {}", json_string),
                        Err(e) => eprintln!("Failed to serialize TR101290 Errors to JSON: {}", e),
                    }

                    batch.push(stream_data.clone());

                    // Check if batch is full
                    if batch.len() >= batch_size {
                        // Send the batch to the channel
                        tx.send(batch.clone()).unwrap();
                        batch.clear();
                    }
                }
            }
            Err(e) => {
                error!("Error capturing packet: {:?}", e);
                break; // or handle the error as needed
            }
        }
    }

    println!("Exiting rscap probe");

    // Send stop signal
    tx.send(Vec::new()).unwrap();
    drop(tx);

    // Wait for the zmq_thread to finish
    //ptx.send(Vec::new()).unwrap();
    //drop(ptx);
    zmq_thread.join().unwrap();
    capture_thread.join().unwrap();

}

// Check if the packet is MPEG-TS or SMPTE 2110
fn is_mpegts_or_smpte2110(packet: &[u8]) -> i32 {
    profile_fn!(is_mpegts_or_smpte2110);

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
const RFC_4175_EXT_SEQ_NUM_LEN: usize = 2;
const RFC_4175_HEADER_LEN: usize = 6; // Note: extended sequence number not included

fn get_extended_sequence_number(buf: &[u8]) -> u16 {
    profile_fn!(get_extended_sequence_number);
    ((buf[0] as u16) << 8) | buf[1] as u16
}

fn get_line_length(buf: &[u8]) -> u16 {
    profile_fn!(get_line_length);
    ((buf[0] as u16) << 8) | buf[1] as u16
}

fn get_line_field_id(buf: &[u8]) -> u8 {
    profile_fn!(get_line_field_id);
    buf[2] >> 7
}

fn get_line_number(buf: &[u8]) -> u16 {
    profile_fn!(get_line_number);
    ((buf[2] as u16 & 0x7f) << 8) | buf[3] as u16
}

fn get_line_continuation(buf: &[u8]) -> u8 {
    profile_fn!(get_line_continuation);
    buf[4] >> 7
}

fn get_line_offset(buf: &[u8]) -> u16 {
    profile_fn!(get_line_offset);
    ((buf[4] as u16 & 0x7f) << 8) | buf[5] as u16
}
// ## End of RFC 4175 SMPTE2110 header functions ##

// Process the packet and return a vector of SMPTE ST 2110 packets
fn process_smpte2110_packet(
    payload_offset: usize,
    packet: &[u8],
    _packet_size: usize,
    start_time: u64,
) -> Vec<StreamData> {
    profile_fn!(process_smpte2110_packet);
    let start = payload_offset;
    let mut streams = Vec::new();

    if packet.len() > start + 12 {
        if packet[start] == 0x80 || packet[start] == 0x81 {
            let rtp_packet = &packet[start..];

            // Create an RtpReader
            if let Ok(rtp) = RtpReader::new(rtp_packet) {
                // Extract the timestamp
                let timestamp = rtp.timestamp();

                let payload_type = rtp.payload_type();

                let payload_offset = rtp.payload_offset();

                // size of rtp payload
                let chunk_size = rtp_packet.len() - payload_offset;

                let line_length = get_line_length(rtp_packet);
                let line_number = get_line_number(rtp_packet);
                let extended_sequence_number = get_extended_sequence_number(rtp_packet);
                let line_offset = get_line_offset(rtp_packet);
                let field_id = get_line_field_id(rtp_packet);

                let line_continuation = get_line_continuation(rtp_packet);

                let smpte_header_info = json!({
                    "size": chunk_size,
                    "timestamp": timestamp,
                    "payload_type": payload_type,
                    "line_length": line_length,
                    "line_number": line_number,
                    "line_offset": line_offset,
                    "field_id": field_id,
                    "line_continuation": line_continuation,
                });

                let pid = payload_type as u16; /* FIXME */
                let stream_type = payload_type.to_string(); /* FIXME */
                let mut stream_data = StreamData::new(
                    &rtp_packet[payload_offset..],
                    pid,
                    stream_type,
                    start_time,
                    timestamp as u64,
                    0, /* fix me */
                );
                // update streams details in stream_data structure
                stream_data.update_stats(chunk_size, current_unix_timestamp_ms().unwrap_or(0));
                stream_data.set_rtp_fields(
                    timestamp,
                    payload_type,
                    payload_type.to_string(),
                    line_number,
                    line_offset,
                    line_length,
                    field_id,
                );

                streams.push(stream_data);

                //smpte2110_packets.push(rtp_packet.to_vec());
                debug!(
                    "SMPTE ST 2110 Header Info: {}",
                    smpte_header_info.to_string()
                );
            } else {
                hexdump(&packet);
                error!("Error parsing RTP header, not SMPTE ST 2110");
            }
        } else {
            hexdump(&packet);
            error!("No RTP header detected, not SMPTE ST 2110");
        }
    } else {
        hexdump(&packet);
        error!("Packet too small, not SMPTE ST 2110");
    }

    streams
}

// Process the packet and return a vector of MPEG-TS packets
fn process_mpegts_packet(
    payload_offset: usize,
    packet: &[u8],
    packet_size: usize,
    start_time: u64,
) -> Vec<StreamData> {
    profile_fn!(process_mpegts_packet);
    let mut start = payload_offset;
    let mut read_size = packet_size;
    let mut streams = Vec::new();

    while start + read_size <= packet.len() {
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
                chunk,
                pid,
                stream_type,
                start_time,
                timestamp,
                continuity_counter,
            );
            stream_data.update_stats(chunk.len(), current_unix_timestamp_ms().unwrap_or(0));
            streams.push(stream_data);
        } else {
            error!("ProcessPacket: Not MPEG-TS");
            hexdump(&packet);
            read_size = 1; // Skip to the next byte
        }
        start += read_size;
    }

    streams
}

// Print a hexdump of the packet
fn hexdump(packet: &[u8]) {
    profile_fn!(hexdump);
    let pid = extract_pid(packet);
    println!("--------------------------------------------------");
    // print in rows of 16 bytes
    println!("PacketDump: PID {} Packet length: {}", pid, packet.len());
    let mut packet_dump = String::new();
    for (i, chunk) in packet.iter().take(packet.len()).enumerate() {
        if i % 16 == 0 {
            packet_dump.push_str(&format!("\n{:04x}: ", i));
        }
        packet_dump.push_str(&format!("{:02x} ", chunk));
    }
    println!("{}", packet_dump);
    println!("");
    println!("--------------------------------------------------");
}
