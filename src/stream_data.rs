/*
 * stream_data.rs
 *
 * Data structure for the stream data
*/

use crate::current_unix_timestamp_ms;
use ahash::AHashMap;
use lazy_static::lazy_static;
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fmt, sync::Arc, sync::Mutex};

const IAT_CAPTURE_WINDOW_SIZE: usize = 3;

lazy_static! {
    static ref PID_MAP: RwLock<AHashMap<u16, Arc<StreamData>>> = RwLock::new(AHashMap::new());
    static ref IAT_CAPTURE_WINDOW: Mutex<VecDeque<u64>> =
        Mutex::new(VecDeque::with_capacity(IAT_CAPTURE_WINDOW_SIZE));
    static ref IAT_CAPTURE_PEAK: Mutex<u64> = Mutex::new(0);
}

// constant for PAT PID
pub const PAT_PID: u16 = 0;
pub const TS_PACKET_SIZE: usize = 188;

pub struct PatEntry {
    pub program_number: u16,
    pub pmt_pid: u16,
}

pub struct PmtEntry {
    pub stream_pid: u16,
    pub stream_type: u8, // Stream type (e.g., 0x02 for MPEG video)
    pub program_number: u16,
}

pub struct Pmt {
    pub entries: Vec<PmtEntry>,
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
    pub stream_type_number: u8,
    pub capture_time: u64,
    pub capture_iat: u64,
    pub source_ip: String,
    pub source_port: i32,
    pub capture_iat_max: u64,
    pub probe_id: String,
    pub pid_map: String,
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
            stream_type_number: self.stream_type_number,
            capture_time: self.capture_time,
            capture_iat: self.capture_iat,
            source_ip: self.source_ip.to_owned(),
            source_port: self.source_port,
            capture_iat_max: self.capture_iat_max,
            probe_id: self.probe_id.to_owned(),
            pid_map: self.pid_map.to_owned(),
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
            stream_type_number,
            capture_time: capture_timestamp,
            capture_iat,
            source_ip,
            source_port,
            capture_iat_max: capture_iat,
            probe_id,
            pid_map: "".to_string(),
            pts: 0,
            pcr: 0,
        }
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
}

// Helper function to parse PAT and update global PAT packet storage
pub fn parse_and_store_pat(packet: &[u8]) -> PmtInfo {
    let pat_entries = parse_pat(packet);
    let mut pmt_info = PmtInfo {
        pid: 0xFFFF,
        packet: Vec::new(),
    };
    pmt_info.packet = packet.to_vec();

    // loook for the program that is non zero and below 0x1FFF
    for entry in pat_entries {
        if entry.pmt_pid != 0 && entry.pmt_pid < 0x1FFF {
            pmt_info.pid = entry.pmt_pid;
            break;
        }
    }
    pmt_info
}

pub fn parse_pat(packet: &[u8]) -> Vec<PatEntry> {
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

pub fn parse_pmt(packet: &[u8]) -> Pmt {
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
            program_number,
        });
        debug!(
            "ParsePMT: ProgramNumber: {}, Stream PID: {}, Stream Type: {}",
            program_number, stream_pid, stream_type
        );
    }

    Pmt { entries }
}

// Invoke this function for each MPEG-TS packet
pub fn process_packet(stream_data_packet: &mut StreamData, pmt_pid: u16, probe_id: String) {
    let packet: &[u8] = &stream_data_packet.packet[stream_data_packet.packet_start
        ..stream_data_packet.packet_start + stream_data_packet.packet_len];

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

pub fn cleanup_stale_streams() {
    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis() as u64;

    let one_minute = 60_000; // 1 minute in milliseconds

    let mut pid_map = PID_MAP.write().unwrap();
    let stale_pids: Vec<u16> = pid_map
        .iter()
        .filter_map(|(&pid, stream_data)| {
            if current_time.saturating_sub(stream_data.last_arrival_time) > one_minute {
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
        if extract_pid(pmt_packet) == pmt_pid {
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
        } else {
            error!("UpdatePIDmap: Skipping PMT PID: {} as it does not match with current PMT packet PID", pmt_pid);
        }
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
