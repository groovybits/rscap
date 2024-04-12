use std::fs::File;
use std::io::Read;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub fn watch_daemon(file_path: &str, sender: Sender<String>, running: Arc<AtomicBool>) {
    // Initialize last_offset to the current end of the file.
    let mut last_offset = match File::open(file_path) {
        Ok(file) => file.metadata().map_or(0, |m| m.len()),
        Err(_) => 0,
    };

    while running.load(Ordering::SeqCst) {
        if let Ok(file) = File::open(file_path) {
            let mut reader = BufReader::new(file);
            // Use seek with the reader.
            if reader.seek(SeekFrom::Start(last_offset)).is_ok() {
                let mut new_lines = Vec::new();
                for line in reader.by_ref().lines().filter_map(Result::ok) {
                    new_lines.push(line);
                }
                if !new_lines.is_empty() {
                    for line in new_lines {
                        if let Err(e) = sender.send(line.clone()) {
                            log::error!("Failed to send line, trying again: {:?}", e);
                            // Retry sending the line.
                            if let Err(e) = sender.send(line) {
                                log::error!("Failed to send line, giving up: {:?}", e);
                            }
                        }
                    }
                    // Update last_offset for the next iteration.
                    last_offset = reader.stream_position().unwrap_or(last_offset);
                }
            }
        }
        // Sleep to reduce CPU usage.
        thread::sleep(Duration::from_secs(1));
    }
}
