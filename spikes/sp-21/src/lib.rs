//! SP-21 (#34): `sqlite-vec` inside the rusqlite `bundled-sqlcipher` build.
//!
//! One encrypted SQLite file holds the items, the outbox, a `vec0` index and an
//! f16 cache column. Embeddings come from an `Embedder`; when it is offline the
//! jobs wait in the outbox and `replay` drains them later.

use std::cell::Cell;
use std::path::Path;
use std::sync::Once;

use half::f16;
use rusqlite::{Connection, Result, params};

/// Vector width (Qwen3 MRL truncation, MEM-21a).
pub const DIM: usize = 256;
/// Value of the `embedding_model` column (MEM-20a/20b).
pub const MODEL: &str = "qwen3-embedding-0.6b@256";

/// `vec0` element type. sqlite-vec 0.1.9 has float32, int8 and bit; no float16.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elem {
    F32,
    Int8,
}

impl Elem {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "f32" => Some(Self::F32),
            "i8" => Some(Self::Int8),
            _ => None,
        }
    }
}

static REGISTER: Once = Once::new();

/// Register the statically linked sqlite-vec for every new connection.
pub fn register_vec() {
    REGISTER.call_once(|| unsafe {
        #[allow(clippy::missing_transmute_annotations)]
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
}

fn hex(key: &[u8; 32]) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}

/// Open (or create) the store with a raw 256-bit key, WAL and `secure_delete`.
/// Fails with SQLITE_NOTADB when the key is wrong.
pub fn open(path: &Path, key: &[u8; 32]) -> Result<Connection> {
    register_vec();
    let conn = Connection::open(path)?;
    // A raw key skips the PBKDF2 step; the app keeps the key in the keychain (MEM-08b).
    conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", hex(key)))?;
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })?;
    conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get::<_, String>(0))?;
    conn.execute_batch("PRAGMA secure_delete = ON; PRAGMA synchronous = NORMAL;")?;
    // SQLCipher turns mmap off for encrypted files, so every page a query reads
    // that is not in the page cache costs a decrypt. `SP21_CACHE_KIB` sizes the
    // page cache; the default 64 MiB holds the whole 50k index, so a warm query
    // decrypts nothing. `SP21_CACHE_KIB=2000` gives SQLite's own default.
    let kib = std::env::var("SP21_CACHE_KIB")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(65_536);
    conn.execute_batch(&format!("PRAGMA cache_size = -{kib};"))?;
    Ok(conn)
}

/// `vec0` column declaration for an element type.
pub fn vec0_decl(elem: Elem) -> String {
    let ty = match elem {
        Elem::F32 => "float",
        Elem::Int8 => "int8",
    };
    // L2 on unit vectors ranks exactly like cosine (|a-b|^2 = 2 - 2cos), and
    // sqlite-vec 0.1.9 has NEON code for L2 but a scalar loop for cosine.
    // `SP21_METRIC=cosine` (bench knob) measures the cosine metric.
    let metric = std::env::var("SP21_METRIC").unwrap_or_else(|_| "l2".into());
    // `SP21_MODEL_COL=metadata` (bench knob) makes `embedding_model` a metadata
    // column instead of a partition key, to compare the filter cost.
    let kind = match std::env::var("SP21_MODEL_COL").as_deref() {
        Ok("metadata") => "",
        _ => " partition key",
    };
    format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS item_vec USING vec0(\
         embedding {ty}[{DIM}] distance_metric={metric}, embedding_model text{kind})"
    )
}

pub fn create_schema(conn: &Connection, elem: Elem) -> Result<()> {
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS items(
           id INTEGER PRIMARY KEY,
           mog_id TEXT NOT NULL,
           text TEXT NOT NULL,
           embedding_model TEXT,
           emb_f16 BLOB);
         CREATE TABLE IF NOT EXISTS outbox(
           id INTEGER PRIMARY KEY,
           kind TEXT NOT NULL,
           item_id INTEGER NOT NULL REFERENCES items(id),
           status TEXT NOT NULL DEFAULT 'pending',
           attempts INTEGER NOT NULL DEFAULT 0);
         CREATE INDEX IF NOT EXISTS outbox_pending ON outbox(status, id);
         CREATE TABLE IF NOT EXISTS meta(k TEXT PRIMARY KEY, v TEXT);
         INSERT OR REPLACE INTO meta VALUES('elem', '{}');
         {};",
        match elem {
            Elem::F32 => "f32",
            Elem::Int8 => "i8",
        },
        vec0_decl(elem)
    ))
}

fn elem_of(conn: &Connection) -> Result<Elem> {
    let s: String = conn.query_row("SELECT v FROM meta WHERE k = 'elem'", [], |r| r.get(0))?;
    Ok(Elem::parse(&s).unwrap_or(Elem::F32))
}

// ---- deterministic helpers -------------------------------------------------

pub fn fnv1a(bytes: &[u8], mut h: u64) -> u64 {
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}
pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// SplitMix64 PRNG (no extra crate).
pub struct Rng(pub u64);
impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    pub fn unit(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }
    pub fn gauss(&mut self) -> f32 {
        let (u, v) = (self.unit(), self.unit());
        ((-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()) as f32
    }
    /// Random L2-normalised vector of width DIM.
    pub fn unit_vec(&mut self) -> Vec<f32> {
        let mut v: Vec<f32> = (0..DIM).map(|_| self.gauss()).collect();
        normalise(&mut v);
        v
    }
}

pub fn normalise(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut().for_each(|x| *x /= n);
}

/// Deterministic embedder double: the vector depends only on the text.
pub fn embed(text: &str) -> Vec<f32> {
    Rng(fnv1a(text.as_bytes(), FNV_OFFSET)).unit_vec()
}

pub fn f32_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}
pub fn i8_blob(v: &[f32]) -> Vec<u8> {
    v.iter()
        .map(|x| ((x * 127.0).round().clamp(-127.0, 127.0) as i8) as u8)
        .collect()
}
pub fn f16_blob(v: &[f32]) -> Vec<u8> {
    v.iter()
        .flat_map(|x| f16::from_f32(*x).to_le_bytes())
        .collect()
}

/// Bytes the vec0 index stores for one vector.
pub fn vec_blob(elem: Elem, v: &[f32]) -> Vec<u8> {
    match elem {
        Elem::F32 => f32_blob(v),
        Elem::Int8 => i8_blob(v),
    }
}

fn vec_param(elem: Elem) -> &'static str {
    match elem {
        Elem::F32 => "?",
        Elem::Int8 => "vec_int8(?)",
    }
}

// ---- embedder and outbox ---------------------------------------------------

pub trait Embedder {
    /// `None` means offline.
    fn embed(&self, text: &str) -> Option<Vec<f32>>;
}

/// Test double for the Databricks embedding endpoint (MEM-19a).
pub struct TestEmbedder {
    online: Cell<bool>,
    calls: Cell<usize>,
}

impl TestEmbedder {
    pub fn new(online: bool) -> Self {
        Self {
            online: Cell::new(online),
            calls: Cell::new(0),
        }
    }
    pub fn set_online(&self, online: bool) {
        self.online.set(online);
    }
    pub fn calls(&self) -> usize {
        self.calls.get()
    }
}

impl Embedder for TestEmbedder {
    fn embed(&self, text: &str) -> Option<Vec<f32>> {
        if !self.online.get() {
            return None;
        }
        self.calls.set(self.calls.get() + 1);
        Some(embed(text))
    }
}

/// Write the item row and its outbox job in one transaction (MEM-10a, MEM-19d).
pub fn remember_offline(conn: &Connection, mog_id: &str, text: &str) -> Result<i64> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO items(mog_id, text) VALUES (?1, ?2)",
        params![mog_id, text],
    )?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO outbox(kind, item_id) VALUES ('embed', ?1)",
        [id],
    )?;
    tx.commit()?;
    Ok(id)
}

pub fn pending(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT count(*) FROM outbox WHERE status = 'pending'",
        [],
        |r| r.get(0),
    )
}

/// Drain pending embed jobs in FIFO batches. Each batch is one transaction:
/// delete + insert the vec0 row (idempotent by rowid), write the f16 cache,
/// mark the job done. Returns the number of jobs done. Stops at the first
/// offline answer and leaves the rest pending.
///
/// `crash_after = Some(n)` calls `process::exit(3)` right after the n-th row
/// is written, inside an open transaction (fault injection).
pub fn replay(
    conn: &Connection,
    emb: &dyn Embedder,
    batch: usize,
    crash_after: Option<usize>,
) -> Result<usize> {
    let elem = elem_of(conn)?;
    let insert_sql = format!(
        "INSERT INTO item_vec(rowid, embedding, embedding_model) VALUES (?1, {}, ?3)",
        vec_param(elem).replace('?', "?2")
    );
    let mut done = 0usize;
    loop {
        let jobs: Vec<(i64, i64, String)> = {
            let mut st = conn.prepare_cached(
                "SELECT o.id, o.item_id, i.text FROM outbox o JOIN items i ON i.id = o.item_id
                 WHERE o.status = 'pending' AND o.kind = 'embed' ORDER BY o.id LIMIT ?1",
            )?;
            st.query_map([batch as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<_>>()?
        };
        if jobs.is_empty() {
            return Ok(done);
        }
        // Network calls happen outside the write transaction.
        let mut vecs = Vec::with_capacity(jobs.len());
        for (_, _, text) in &jobs {
            match emb.embed(text) {
                Some(v) => vecs.push(v),
                None => break,
            }
        }
        if vecs.is_empty() {
            return Ok(done);
        }
        let offline_mid_batch = vecs.len() < jobs.len();
        let tx = conn.unchecked_transaction()?;
        {
            let mut del = tx.prepare_cached("DELETE FROM item_vec WHERE rowid = ?1")?;
            let mut ins = tx.prepare_cached(&insert_sql)?;
            let mut upd = tx.prepare_cached(
                "UPDATE items SET emb_f16 = ?2, embedding_model = ?3 WHERE id = ?1",
            )?;
            let mut fin = tx.prepare_cached(
                "UPDATE outbox SET status = 'done', attempts = attempts + 1 WHERE id = ?1",
            )?;
            for ((job_id, item_id, _), v) in jobs.iter().zip(&vecs) {
                del.execute([item_id])?;
                ins.execute(params![item_id, vec_blob(elem, v), MODEL])?;
                upd.execute(params![item_id, f16_blob(v), MODEL])?;
                fin.execute([job_id])?;
                done += 1;
                if crash_after == Some(done) {
                    std::process::exit(3);
                }
            }
        }
        tx.commit()?;
        if offline_mid_batch {
            return Ok(done);
        }
    }
}

// ---- audit -----------------------------------------------------------------

#[derive(Debug, Default, PartialEq, Eq)]
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

/// Count and checksum what is stored against what the embedder would produce
/// for every item whose job is done.
pub fn audit(conn: &Connection) -> Result<Audit> {
    let elem = elem_of(conn)?;
    let one = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0));
    let mut a = Audit {
        items: one("SELECT count(*) FROM items")?,
        pending: pending(conn)?,
        vec_rows: one("SELECT count(*) FROM item_vec")?,
        f16_rows: one("SELECT count(*) FROM items WHERE emb_f16 IS NOT NULL")?,
        ..Default::default()
    };
    let mut ids = Vec::new();
    let (mut vc, mut fc) = (FNV_OFFSET, FNV_OFFSET);
    {
        let mut st = conn.prepare("SELECT rowid, embedding FROM item_vec ORDER BY rowid")?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let id: i64 = r.get(0)?;
            let b: Vec<u8> = r.get(1)?;
            ids.push(id);
            vc = fnv1a(&b, fnv1a(&id.to_le_bytes(), vc));
        }
    }
    let mut dedup = ids.clone();
    dedup.dedup();
    a.distinct_vec_rowids = dedup.len() as i64;
    a.vec_checksum = vc;
    {
        let mut st =
            conn.prepare("SELECT id, emb_f16 FROM items WHERE emb_f16 IS NOT NULL ORDER BY id")?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let id: i64 = r.get(0)?;
            let b: Vec<u8> = r.get(1)?;
            fc = fnv1a(&b, fnv1a(&id.to_le_bytes(), fc));
        }
    }
    a.f16_checksum = fc;
    let (mut ev, mut ef) = (FNV_OFFSET, FNV_OFFSET);
    let mut st = conn.prepare(
        "SELECT i.id, i.text FROM items i
         WHERE EXISTS (SELECT 1 FROM outbox o WHERE o.item_id = i.id AND o.status = 'done')
         ORDER BY i.id",
    )?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        let text: String = r.get(1)?;
        let v = embed(&text);
        ev = fnv1a(&vec_blob(elem, &v), fnv1a(&id.to_le_bytes(), ev));
        ef = fnv1a(&f16_blob(&v), fnv1a(&id.to_le_bytes(), ef));
    }
    a.expected_vec_checksum = ev;
    a.expected_f16_checksum = ef;
    Ok(a)
}

// ---- recall ----------------------------------------------------------------

/// vec0 KNN, filtered on `embedding_model` (MEM-20b).
pub fn knn(conn: &Connection, elem: Elem, q: &[f32], k: usize) -> Result<Vec<(i64, f64)>> {
    // `SP21_NO_FILTER=1` (bench knob) drops the metadata filter to measure its cost.
    let filter = std::env::var_os("SP21_NO_FILTER").is_none();
    let sql = format!(
        "SELECT rowid, distance FROM item_vec WHERE embedding MATCH {} AND k = ?2{}",
        vec_param(elem).replace('?', "?1"),
        if filter {
            " AND embedding_model = ?3"
        } else {
            ""
        }
    );
    let mut st = conn.prepare_cached(&sql)?;
    let map = |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?));
    if filter {
        st.query_map(params![vec_blob(elem, q), k as i64, MODEL], map)?
            .collect()
    } else {
        st.query_map(params![vec_blob(elem, q), k as i64], map)?
            .collect()
    }
}

/// Two-stage recall: int8 vec0 KNN for `k * over` candidates, then exact cosine
/// on their f16 cache rows. Returns (id, cosine distance) best first.
pub fn knn_int8_rerank(
    conn: &Connection,
    q: &[f32],
    k: usize,
    over: usize,
) -> Result<Vec<(i64, f32)>> {
    let cands = knn(conn, Elem::Int8, q, k * over)?;
    let mut st = conn.prepare_cached("SELECT emb_f16 FROM items WHERE id = ?1")?;
    let mut out = Vec::with_capacity(cands.len());
    for (id, _) in cands {
        let b: Vec<u8> = st.query_row([id], |r| r.get(0))?;
        let dot: f32 = b
            .as_chunks::<2>()
            .0
            .iter()
            .zip(q)
            .map(|(p, x)| f16::from_le_bytes(*p).to_f32() * x)
            .sum();
        out.push((id, 1.0 - dot));
    }
    out.sort_by(|a, b| a.1.total_cmp(&b.1));
    out.truncate(k);
    Ok(out)
}

/// In-memory copy of the f16 cache column, row-major.
pub struct F16Cache {
    pub ids: Vec<i64>,
    pub data: Vec<f16>,
}

pub fn load_f16_cache(conn: &Connection) -> Result<F16Cache> {
    let mut c = F16Cache {
        ids: Vec::new(),
        data: Vec::new(),
    };
    let mut st = conn.prepare(
        "SELECT id, emb_f16 FROM items WHERE emb_f16 IS NOT NULL AND embedding_model = ?1",
    )?;
    let mut rows = st.query([MODEL])?;
    while let Some(r) = rows.next()? {
        c.ids.push(r.get(0)?);
        let b: Vec<u8> = r.get(1)?;
        c.data
            .extend(b.as_chunks::<2>().0.iter().map(|p| f16::from_le_bytes(*p)));
    }
    Ok(c)
}

/// Fallback recall: brute-force cosine over the f16 cache (vectors are unit
/// length, so cosine = dot). Returns (id, cosine distance) best first.
pub fn brute_force(cache: &F16Cache, q: &[f32], k: usize) -> Vec<(i64, f32)> {
    let mut top: Vec<(i64, f32)> = Vec::with_capacity(k + 1);
    for (i, row) in cache.data.as_chunks::<DIM>().0.iter().enumerate() {
        let dot: f32 = row.iter().zip(q).map(|(a, b)| a.to_f32() * b).sum();
        let d = 1.0 - dot;
        if top.len() < k || d < top[top.len() - 1].1 {
            let pos = top.partition_point(|t| t.1 <= d);
            top.insert(pos, (cache.ids[i], d));
            top.truncate(k);
        }
    }
    top
}

/// Nearest-rank percentile of a sorted slice.
pub fn pct(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    let idx = ((p / 100.0) * n as f64).ceil() as usize;
    sorted[idx.clamp(1, n) - 1]
}
