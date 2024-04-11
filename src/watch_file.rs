use std::fs::File;
use std::io::Read;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

pub fn watch_daemon(file_path: &str, sender: Sender<String>) {
    let mut last_offset = 0;

    loop {
        if let Ok(file) = File::open(file_path) {
            let mut reader = BufReader::new(file);
            reader.seek(SeekFrom::Start(last_offset)).unwrap();

            let mut new_lines = Vec::new();
            let lines = reader.by_ref().lines().filter_map(Result::ok);
            for line in lines {
                new_lines.push(line);
            }

            if !new_lines.is_empty() {
                for line in new_lines {
                    sender.send(line).unwrap();
                }
                last_offset = reader.stream_position().unwrap();
            }
        }

        thread::sleep(Duration::from_secs(1));
    }
}
