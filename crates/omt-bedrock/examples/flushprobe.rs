//! Probe: how many tables does a flush of ~10 MB produce?
fn main() {
    let dir = std::env::temp_dir().join(format!("omt-flushprobe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let db = bedrock_leveldb::Db::open(&dir, bedrock_leveldb::OpenOptions { write_buffer_size: 0, ..Default::default() }).expect("open");
    let mut batch = bedrock_leveldb::WriteBatch::new();
    for i in 0u32..10_000 {
        batch.put(format!("key{i:08}").into_bytes(), vec![(i % 251) as u8; 1000]);
    }
    db.write(batch, bedrock_leveldb::WriteOptions::default()).expect("write");
    db.flush().expect("flush");
    drop(db);
    let files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| (e.file_name(), e.metadata().unwrap().len())).collect();
    println!("{files:?}");
    let db = bedrock_leveldb::Db::open(&dir, bedrock_leveldb::OpenOptions { read_only: true, ..Default::default() }).expect("reopen");
    let t = std::time::Instant::now();
    for i in (0u32..10_000).step_by(50) {
        assert!(db.get(format!("key{i:08}").as_bytes()).unwrap().is_some());
    }
    println!("200 gets: {:?}", t.elapsed());
    let _ = std::fs::remove_dir_all(&dir);
}
