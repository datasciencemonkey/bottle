//! SP-21 (#34) red stage: the API the tests need. Bodies land in the green commit.
#![allow(unused_variables)]

use std::cell::Cell;
use std::path::Path;

use half::f16;
use rusqlite::{Connection, Result};

#[derive(Clone, Copy, Debug)]
pub enum Elem {
    F32,
    Int8,
}

pub struct TestEmbedder {
    online: Cell<bool>,
}

impl TestEmbedder {
    pub fn new(online: bool) -> Self {
        Self {
            online: Cell::new(online),
        }
    }
    pub fn set_online(&self, online: bool) {
        self.online.set(online);
    }
}

#[derive(Debug, Default)]
pub struct Audit {
    pub items: i64,
    pub pending: i64,
    pub vec_rows: i64,
    pub distinct_vec_rowids: i64,
    pub f16_rows: i64,
    pub vec_checksum: u64,
    pub expected_vec_checksum: u64,
    pub f16_checksum: u64,
    pub expected_f16_checksum: u64,
}

pub struct F16Cache {
    pub ids: Vec<i64>,
    pub data: Vec<f16>,
}

pub fn open(path: &Path, key: &[u8; 32]) -> Result<Connection> {
    todo!("TODO(#34) green")
}
pub fn create_schema(conn: &Connection, elem: Elem) -> Result<()> {
    todo!("TODO(#34) green")
}
pub fn embed(text: &str) -> Vec<f32> {
    todo!("TODO(#34) green")
}
pub fn remember_offline(conn: &Connection, mog_id: &str, text: &str) -> Result<i64> {
    todo!("TODO(#34) green")
}
pub fn replay(
    conn: &Connection,
    emb: &TestEmbedder,
    batch: usize,
    crash_after: Option<usize>,
) -> Result<usize> {
    todo!("TODO(#34) green")
}
pub fn pending(conn: &Connection) -> Result<i64> {
    todo!("TODO(#34) green")
}
pub fn audit(conn: &Connection) -> Result<Audit> {
    todo!("TODO(#34) green")
}
pub fn knn(conn: &Connection, elem: Elem, q: &[f32], k: usize) -> Result<Vec<(i64, f64)>> {
    todo!("TODO(#34) green")
}
pub fn load_f16_cache(conn: &Connection) -> Result<F16Cache> {
    todo!("TODO(#34) green")
}
pub fn brute_force(cache: &F16Cache, q: &[f32], k: usize) -> Vec<(i64, f32)> {
    todo!("TODO(#34) green")
}
