use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom}; // Corrected imports
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

pub fn watch_daemon(file_path: &str, sender: Sender<String>) {
    let initial_offset = match File::open(file_path) {
        Ok(file) => file.metadata().map(|m| m.len() as u64).unwrap_or(0),
        Err(_) => 0,
    };

    let mut last_offset = initial_offset;

    loop {
        if let Ok(file) = File::open(file_path) {
            let mut reader = BufReader::new(file);

            if reader.seek(SeekFrom::Start(last_offset)).is_ok() {
                let mut new_lines = Vec::new();
                // Use `by_ref` to prevent moving `reader`
                let lines = reader.by_ref().lines().filter_map(Result::ok);
                for line in lines {
                    new_lines.push(line);
                }

                if !new_lines.is_empty() {
                    for line in new_lines {
                        if let Err(e) = sender.send(line) {
                            eprintln!("Failed to send line: {:?}", e);
                        }
                    }
                    // `reader` can be reused here because it wasn't moved
                    last_offset = reader.stream_position().unwrap_or(last_offset);
                }
            }
        }
        thread::sleep(Duration::from_secs(1));
    }
}
