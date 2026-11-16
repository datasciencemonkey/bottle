//! SP-21 (#34) bench and fault-injection helper.
//!
//! bench build   <db> <n> <f32|i8> [seed]   insert n random unit vectors (vec0 + f16 cache)
//! bench query   <db> <k> <nq> [seed]       vec0 KNN latency; the first query is the cold one
//! bench brute   <db> <k> <nq> [seed]       brute-force cosine over the f16 cache
//! bench recall  <f32db> <i8db> <k> <nq>    recall@k of int8 vec0 and f16 brute vs f32 vec0
//! bench check   <dir>                      cipher_version, vec_version and one KNN
//! bench replay-crash <db> <batch> <n>      replay, exit(3) after row n (used by the tests)

use std::path::Path;
use std::time::Instant;

use rusqlite::{Connection, params};
use sp21::*;

/// Fixed spike key. The app uses a random 256-bit key from the keychain (MEM-08b).
const KEY: [u8; 32] = [7u8; 32];

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn stats(label: &str, mut v: Vec<f64>) {
    if v.is_empty() {
        return;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "{label}: n={} p50={:.3}ms p95={:.3}ms p99={:.3}ms max={:.3}ms",
        v.len(),
        pct(&v, 50.0),
        pct(&v, 95.0),
        pct(&v, 99.0),
        v[v.len() - 1]
    );
}

fn arg<T: std::str::FromStr>(a: &[String], i: usize, d: T) -> T {
    a.get(i).and_then(|s| s.parse().ok()).unwrap_or(d)
}

fn build(path: &Path, n: usize, elem: Elem, seed: u64) -> rusqlite::Result<()> {
    let _ = std::fs::remove_file(path);
    let conn = open(path, &KEY)?;
    create_schema(&conn, elem)?;
    let mut rng = Rng(seed);
    let ins_vec = format!(
        "INSERT INTO item_vec(rowid, embedding, embedding_model) VALUES (?1, {}, ?3)",
        if elem == Elem::Int8 {
            "vec_int8(?2)"
        } else {
            "?2"
        }
    );
    let t = Instant::now();
    for chunk in 0..n.div_ceil(1000) {
        let tx = conn.unchecked_transaction()?;
        {
            let mut it = tx.prepare_cached(
                "INSERT INTO items(id, mog_id, text, embedding_model, emb_f16) VALUES (?1, 'mog-a', ?2, ?3, ?4)",
            )?;
            let mut iv = tx.prepare_cached(&ins_vec)?;
            for i in chunk * 1000..((chunk + 1) * 1000).min(n) {
                let v = rng.unit_vec();
                let id = i as i64 + 1;
                it.execute(params![id, format!("item {i}"), MODEL, f16_blob(&v)])?;
                iv.execute(params![id, vec_blob(elem, &v), MODEL])?;
            }
        }
        tx.commit()?;
    }
    let insert_ms = ms(t);
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    println!(
        "build {elem:?}: n={n} insert_total={:.0}ms per_row={:.3}ms file={} bytes ({:.2} MB)",
        insert_ms,
        insert_ms / n as f64,
        size,
        size as f64 / 1e6
    );
    // Per-table bytes (dbstat works on the decrypted pages).
    let mut st = conn.prepare(
        "SELECT CASE WHEN name LIKE 'item_vec%' THEN 'vec0 (item_vec*)'
                     WHEN name = 'items' THEN 'items (text + f16 cache)'
                     ELSE 'other' END AS g, sum(pgsize)
         FROM dbstat GROUP BY g ORDER BY g",
    )?;
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    for r in rows {
        let (g, b) = r?;
        println!("  {g}: {b} bytes ({:.2} MB)", b as f64 / 1e6);
    }
    Ok(())
}

fn elem_of_db(conn: &Connection) -> Elem {
    let s: String = conn
        .query_row("SELECT v FROM meta WHERE k='elem'", [], |r| r.get(0))
        .unwrap();
    Elem::parse(&s).unwrap()
}

fn query(path: &Path, k: usize, nq: usize, seed: u64) -> rusqlite::Result<()> {
    let t = Instant::now();
    let conn = open(path, &KEY)?;
    let open_ms = ms(t);
    let elem = elem_of_db(&conn);
    let mut rng = Rng(seed);
    let q = rng.unit_vec();
    let t = Instant::now();
    let hits = knn(&conn, elem, &q, k)?;
    let cold = ms(t);
    assert_eq!(hits.len(), k);
    println!("query {elem:?} k={k}: open={open_ms:.3}ms cold_first_query={cold:.3}ms");
    let mut lat = Vec::with_capacity(nq);
    for _ in 0..nq {
        let q = rng.unit_vec();
        let t = Instant::now();
        let hits = knn(&conn, elem, &q, k)?;
        lat.push(ms(t));
        assert_eq!(hits.len(), k);
    }
    stats(&format!("  warm {elem:?} k={k}"), lat);
    Ok(())
}

fn brute(path: &Path, k: usize, nq: usize, seed: u64) -> rusqlite::Result<()> {
    let conn = open(path, &KEY)?;
    let t = Instant::now();
    let cache = load_f16_cache(&conn)?;
    let load = ms(t);
    let mut rng = Rng(seed);
    let q = rng.unit_vec();
    let t = Instant::now();
    let _ = brute_force(&cache, &q, k);
    let cold = ms(t);
    println!(
        "brute f16 k={k}: rows={} cache_load={load:.1}ms cache_bytes={} first_query={cold:.3}ms",
        cache.ids.len(),
        cache.data.len() * 2
    );
    let mut lat = Vec::with_capacity(nq);
    for _ in 0..nq {
        let q = rng.unit_vec();
        let t = Instant::now();
        let r = brute_force(&cache, &q, k);
        lat.push(ms(t));
        assert_eq!(r.len(), k);
    }
    stats(&format!("  warm brute f16 k={k}"), lat);
    Ok(())
}

fn recall(f32db: &Path, i8db: &Path, k: usize, nq: usize) -> rusqlite::Result<()> {
    let a = open(f32db, &KEY)?;
    let b = open(i8db, &KEY)?;
    let cache = load_f16_cache(&a)?;
    let mut rng = Rng(99);
    let (mut hi8, mut hf16, mut hrr) = (0usize, 0usize, 0usize);
    let mut lat = Vec::with_capacity(nq);
    for _ in 0..nq {
        let q = rng.unit_vec();
        let exact: Vec<i64> = knn(&a, Elem::F32, &q, k)?
            .into_iter()
            .map(|h| h.0)
            .collect();
        let i8: Vec<i64> = knn(&b, Elem::Int8, &q, k)?
            .into_iter()
            .map(|h| h.0)
            .collect();
        let f16: Vec<i64> = brute_force(&cache, &q, k)
            .into_iter()
            .map(|h| h.0)
            .collect();
        let t = Instant::now();
        let rr: Vec<i64> = knn_int8_rerank(&b, &q, k, 5)?
            .into_iter()
            .map(|h| h.0)
            .collect();
        lat.push(ms(t));
        hrr += rr.iter().filter(|x| exact.contains(x)).count();
        hi8 += i8.iter().filter(|x| exact.contains(x)).count();
        hf16 += f16.iter().filter(|x| exact.contains(x)).count();
    }
    let d = (nq * k) as f64;
    println!(
        "recall@{k} vs f32 vec0 over {nq} queries: int8 vec0={:.4} int8 x5 + f16 rerank={:.4} f16 brute={:.4}",
        hi8 as f64 / d,
        hrr as f64 / d,
        hf16 as f64 / d
    );
    stats(&format!("  int8 x5 + f16 rerank k={k}"), lat);
    Ok(())
}

fn check(dir: &Path) -> rusqlite::Result<()> {
    std::fs::create_dir_all(dir).ok();
    let path = dir.join("check.db");
    let _ = std::fs::remove_file(&path);
    let conn = open(&path, &KEY)?;
    let cipher: String = conn.query_row("PRAGMA cipher_version", [], |r| r.get(0))?;
    let provider: String = conn.query_row("PRAGMA cipher_provider", [], |r| r.get(0))?;
    let vec: String = conn.query_row("SELECT vec_version()", [], |r| r.get(0))?;
    create_schema(&conn, Elem::F32)?;
    for i in 0..20 {
        remember_offline(&conn, "mog-a", &format!("c {i}"))?;
    }
    replay(&conn, &TestEmbedder::new(true), 10, None)?;
    let hits = knn(&conn, Elem::F32, &embed("c 5"), 1)?;
    drop(conn);
    let head = std::fs::read(&path).unwrap_or_default();
    let plain = head.starts_with(b"SQLite format 3\0");
    println!(
        "cipher_version={cipher} cipher_provider={provider} vec_version={vec} knn_top1_rowid={} header_plaintext={plain}",
        hits[0].0
    );
    Ok(())
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let cmd = a.get(1).map(String::as_str).unwrap_or("");
    let p = |i: usize| Path::new(a.get(i).map(String::as_str).unwrap_or("/tmp/sp21.db")).to_owned();
    let r = match cmd {
        "build" => build(
            &p(2),
            arg(&a, 3, 50_000),
            Elem::parse(a.get(4).map(String::as_str).unwrap_or("f32")).expect("f32|i8"),
            arg(&a, 5, 42),
        ),
        "query" => query(&p(2), arg(&a, 3, 10), arg(&a, 4, 1000), arg(&a, 5, 7)),
        "brute" => brute(&p(2), arg(&a, 3, 10), arg(&a, 4, 1000), arg(&a, 5, 7)),
        "recall" => recall(&p(2), &p(3), arg(&a, 4, 10), arg(&a, 5, 200)),
        "check" => check(&p(2)),
        "replay-crash" => {
            let conn = open(&p(2), &KEY).expect("open");
            replay(
                &conn,
                &TestEmbedder::new(true),
                arg(&a, 3, 100),
                Some(arg(&a, 4, 1)),
            )
            .map(|_| ())
        }
        _ => {
            eprintln!("usage: see the header of src/bin/bench.rs");
            std::process::exit(2);
        }
    };
    if let Err(e) = r {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
