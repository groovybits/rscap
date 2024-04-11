use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom}; // Ensure Read is imported here
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

pub fn watch_daemon(file_path: &str, sender: Sender<String>) {
    // Initialize last_offset to the current end of the file.
    let mut last_offset = match File::open(file_path) {
        Ok(file) => file.metadata().map_or(0, |m| m.len()),
        Err(_) => 0,
    };

    loop {
        if let Ok(file) = File::open(file_path) {
            let mut reader = BufReader::new(file);

            // use seek with the reader.
            if reader.seek(SeekFrom::Start(last_offset)).is_ok() {
                for line in reader.by_ref().lines().filter_map(Result::ok) {
                    if let Err(e) = sender.send(line.clone()) {
                        log::error!("Failed to send line, trying again: {:?}", e);
                        // retry sending the line.
                        if let Err(e) = sender.send(line) {
                            log::error!("Failed to send line, giving up: {:?}", e);
                        }
                    }
                }

                // Update last_offset for the next iteration.
                last_offset = reader.stream_position().unwrap_or(last_offset);
            }
        }
        // Sleep to reduce CPU usage.
        thread::sleep(Duration::from_secs(1));
    }
}
