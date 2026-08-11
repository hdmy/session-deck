use crate::{
    domain::{Result, SearchHit},
    storage,
};
use rusqlite::Connection;
pub fn query(c: &Connection, q: &str) -> Result<Vec<SearchHit>> {
    storage::search(c, q)
}
