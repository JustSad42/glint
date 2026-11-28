//! Shared by the Glint indexer service and the launcher.
//! - `index`: the in-memory file table and ranking (platform independent, unit tested)
//! - `query`: parsing what the user typed into tokens and filters
//! - `proto`: the JSON-lines messages sent over the named pipe
//! - `ntfs`: reading a volume's file table and change journal (Windows only)
//! - `pipe`: the named-pipe server and client (Windows only)
pub mod access;
pub mod index;
pub mod proto;
pub mod query;

#[cfg(windows)]
pub mod ntfs;
#[cfg(windows)]
pub mod pipe;

pub const PIPE_NAME: &str = r"\\.\pipe\glint-index";
pub const SERVICE_NAME: &str = "GlintIndexer";
