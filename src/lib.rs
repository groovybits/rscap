pub mod stream_data;

use std::time::{SystemTime, UNIX_EPOCH};

// Function to get the current Unix timestamp in milliseconds
pub fn current_unix_timestamp_ms() -> Result<u64, &'static str> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .map_err(|_| "System time is before the UNIX epoch")
}
