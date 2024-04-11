use std::fs::File;
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

            {
                let reader = &mut reader;
                let lines: Vec<String> = reader.lines().filter_map(Result::ok).collect();

                for line in lines {
                    sender.send(line).unwrap();
                }
            }

            last_offset = reader.stream_position().unwrap();
        }

        thread::sleep(Duration::from_secs(1));
    }
}
