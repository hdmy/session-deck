use crate::{
    domain::{ParsedSession, Result},
    storage,
};
use rusqlite::Connection;
pub fn reconcile(
    connection: &mut Connection,
    provider_id: &str,
    sessions: &[ParsedSession],
) -> Result<usize> {
    storage::index(connection, provider_id, sessions)
}

pub fn reconcile_incremental(
    connection: &mut Connection,
    provider_id: &str,
    parsed: &[ParsedSession],
    discovered_paths: &[std::path::PathBuf],
) -> Result<usize> {
    storage::index_incremental(connection, provider_id, parsed, discovered_paths)
}
