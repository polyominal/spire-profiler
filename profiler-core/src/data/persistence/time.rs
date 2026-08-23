//! The epoch-seconds clock: [`now_seconds`]. Records carry timestamps as
//! i64 epoch seconds on the wire — the in-memory value serializes directly
//! and nothing on the read path round-trips ISO strings.

use std::time::SystemTime;
pub fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
