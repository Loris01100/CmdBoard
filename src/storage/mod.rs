mod backup;
pub mod db;
mod goals;
pub mod models;
mod queries;
mod sessions;

pub use backup::NewImport;
pub use db::Database;
pub use queries::unix_now;

/// A stored count of seconds or bytes. SQLite integers are signed: a negative one reads as 0.
fn to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// A count of seconds or bytes to store, saturating at SQLite's largest integer.
fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
