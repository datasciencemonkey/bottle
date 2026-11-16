//! SP-21 (#34) tests. Each test name carries the requirement ID.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use sp21::{
    Elem, TestEmbedder, audit, brute_force, create_schema, embed, knn, load_f16_cache, open,
};

const KEY: [u8; 32] = [7u8; 32];

fn tmp_db(name: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "sp21-test-{}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst),
        name
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("memory.db")
}

/// [SP-21] [MEM-08e] sqlite-vec loads into the SQLCipher connection and runs a vec0 KNN.
#[test]
fn sp_21_vec_loads_in_sqlcipher() {
    let path = tmp_db("loads");
    let conn = open(&path, &KEY).unwrap();
    let cipher: String = conn
        .query_row("PRAGMA cipher_version", [], |r| r.get(0))
        .unwrap();
    assert!(cipher.starts_with("4."), "cipher_version = {cipher}");
    let vec: String = conn
        .query_row("SELECT vec_version()", [], |r| r.get(0))
        .unwrap();
    assert_eq!(vec, "v0.1.9");
    create_schema(&conn, Elem::F32).unwrap();
    let emb = TestEmbedder::new(true);
    for i in 0..10 {
        sp21::remember_offline(&conn, "mog-a", &format!("item {i}")).unwrap();
    }
    assert_eq!(sp21::replay(&conn, &emb, 4, None).unwrap(), 10);
    let hits = knn(&conn, Elem::F32, &embed("item 3"), 3).unwrap();
    assert_eq!(hits.len(), 3);
    assert_eq!(
        hits[0].0, 4,
        "item 3 has rowid 4 and must be its own nearest neighbour"
    );
}

/// [SP-21] [MEM-08b] [MEM-08c] The file is encrypted, in WAL mode, and unreadable without the key.
#[test]
fn sp_21_db_is_encrypted() {
    let path = tmp_db("enc");
    {
        let conn = open(&path, &KEY).unwrap();
        create_schema(&conn, Elem::F32).unwrap();
        sp21::remember_offline(&conn, "mog-a", "secret plaintext marker").unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let sd: i64 = conn
            .query_row("PRAGMA secure_delete", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sd, 1);
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
    }
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.len() > 4096);
    assert_ne!(
        &bytes[..16],
        b"SQLite format 3\0",
        "header must not be plaintext"
    );
    let needle = b"secret plaintext marker";
    assert!(
        !bytes.windows(needle.len()).any(|w| w == needle),
        "row text must not appear in the file"
    );
}

/// [SP-21] [MEM-08b] A wrong key and no key both fail to read the file.
#[test]
fn sp_21_wrong_key_fails() {
    let path = tmp_db("wrongkey");
    {
        let conn = open(&path, &KEY).unwrap();
        create_schema(&conn, Elem::F32).unwrap();
    }
    assert!(open(&path, &[8u8; 32]).is_err(), "wrong key must fail");
    let plain = rusqlite::Connection::open(&path).unwrap();
    assert!(
        plain
            .query_row("SELECT count(*) FROM sqlite_master", [], |r| r
                .get::<_, i64>(0))
            .is_err(),
        "no key must fail"
    );
    assert!(open(&path, &KEY).is_ok(), "right key must still work");
}

/// [SP-21] [MEM-19d] [MEM-08f] 1,000 jobs queued offline survive a restart and drain online
/// with no loss and no duplicates (count + checksum).
#[test]
fn sp_21_queue_replay_no_loss() {
    let path = tmp_db("replay");
    let emb = TestEmbedder::new(false);
    {
        let conn = open(&path, &KEY).unwrap();
        create_schema(&conn, Elem::F32).unwrap();
        for i in 0..1000 {
            sp21::remember_offline(&conn, "mog-a", &format!("note {i}")).unwrap();
        }
        // Offline: replay does nothing and keeps every job.
        assert_eq!(sp21::replay(&conn, &emb, 100, None).unwrap(), 0);
        assert_eq!(sp21::pending(&conn).unwrap(), 1000);
    }
    // Restart, then go online.
    let conn = open(&path, &KEY).unwrap();
    assert_eq!(sp21::pending(&conn).unwrap(), 1000);
    emb.set_online(true);
    assert_eq!(sp21::replay(&conn, &emb, 100, None).unwrap(), 1000);
    let a = audit(&conn).unwrap();
    assert_eq!(a.items, 1000);
    assert_eq!(a.pending, 0);
    assert_eq!(a.vec_rows, 1000);
    assert_eq!(a.distinct_vec_rowids, 1000);
    assert_eq!(a.f16_rows, 1000);
    assert_eq!(a.vec_checksum, a.expected_vec_checksum);
    assert_eq!(a.f16_checksum, a.expected_f16_checksum);
}

/// [SP-21] [MEM-10a] [MEM-11a] A process exit mid-replay loses nothing; the restart drains the
/// rest, and a second full replay of every job creates no duplicates.
#[test]
fn sp_21_replay_idempotent_after_crash() {
    let path = tmp_db("crash");
    {
        let conn = open(&path, &KEY).unwrap();
        create_schema(&conn, Elem::F32).unwrap();
        for i in 0..1000 {
            sp21::remember_offline(&conn, "mog-a", &format!("note {i}")).unwrap();
        }
    }
    // Child process: replay in batches of 100, exit(3) after row 437 (inside batch 5).
    let status = Command::new(env!("CARGO_BIN_EXE_bench"))
        .args(["replay-crash", path.to_str().unwrap(), "100", "437"])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(3), "child must die mid-replay");

    let conn = open(&path, &KEY).unwrap();
    let after_crash = audit(&conn).unwrap();
    assert_eq!(after_crash.vec_rows, 400, "the open batch must roll back");
    assert_eq!(after_crash.pending, 600);

    let emb = TestEmbedder::new(true);
    assert_eq!(sp21::replay(&conn, &emb, 100, None).unwrap(), 600);
    let a = audit(&conn).unwrap();
    assert_eq!(
        (a.vec_rows, a.distinct_vec_rowids, a.pending),
        (1000, 1000, 0)
    );
    assert_eq!(a.vec_checksum, a.expected_vec_checksum);

    // At-least-once delivery: mark every job pending again and replay. Nothing doubles.
    conn.execute("UPDATE outbox SET status = 'pending'", [])
        .unwrap();
    assert_eq!(sp21::replay(&conn, &emb, 100, None).unwrap(), 1000);
    let b = audit(&conn).unwrap();
    assert_eq!(
        (b.vec_rows, b.distinct_vec_rowids, b.pending),
        (1000, 1000, 0)
    );
    assert_eq!(b.vec_checksum, a.vec_checksum);
    assert_eq!(b.f16_checksum, b.expected_f16_checksum);
}

/// [SP-21] fallback: brute-force cosine over the f16 cache agrees with the vec0 KNN.
#[test]
fn sp_21_bruteforce_matches_vec0() {
    let path = tmp_db("brute");
    let conn = open(&path, &KEY).unwrap();
    create_schema(&conn, Elem::F32).unwrap();
    for i in 0..2000 {
        sp21::remember_offline(&conn, "mog-a", &format!("doc {i}")).unwrap();
    }
    let emb = TestEmbedder::new(true);
    sp21::replay(&conn, &emb, 500, None).unwrap();
    let cache = load_f16_cache(&conn).unwrap();
    let mut overlap = 0usize;
    for q in 0..50 {
        let qv = embed(&format!("query {q}"));
        let exact: Vec<i64> = knn(&conn, Elem::F32, &qv, 10)
            .unwrap()
            .into_iter()
            .map(|h| h.0)
            .collect();
        let brute: Vec<i64> = brute_force(&cache, &qv, 10)
            .into_iter()
            .map(|h| h.0)
            .collect();
        overlap += brute.iter().filter(|id| exact.contains(id)).count();
    }
    let recall = overlap as f64 / 500.0;
    assert!(recall >= 0.95, "f16 brute-force recall@10 = {recall}");
}
