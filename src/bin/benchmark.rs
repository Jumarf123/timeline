//! Reproducible large-file correctness/performance check, separate from the GUI.
use anyhow::{Result, ensure};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
    sync::Arc,
    time::Instant,
};
use timeline::{
    data::{Dataset, OpenOptionsConfig},
    search::{self, FindSpec, ScanRequest, Scope},
};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("generate") {
        let path = PathBuf::from(args.get(2).expect("path"));
        let mib: u64 = args.get(3).map(|n| n.parse()).transpose()?.unwrap_or(4096);
        let mut file = BufWriter::with_capacity(4 * 1024 * 1024, File::create(path)?);
        file.write_all(b"PID,Process,Path,User,Timestamp,Details\r\n")?;
        let row = b"1234,svchost.exe,C:\\Windows\\System32\\svchost.exe,NT AUTHORITY\\SYSTEM,2026-09-23T11:30:00Z,\"Memory analysis record; normal process activity, with quoted comma and \"\"escaped text\"\" for parser validation\"\r\n";
        let block = row.repeat(8192);
        let blocks = (mib * 1024 * 1024).div_ceil(block.len() as u64);
        for _ in 0..blocks {
            file.write_all(&block)?;
        }
        file.write_all(b"9999,NEEDLE.exe,C:\\evidence\\last.exe,Analyst,2026-09-23T12:00:00Z,\"last record\nwith multiline text\"\r\n")?;
        file.flush()?;
        println!("generated_rows={}", blocks * 8192 + 1);
        return Ok(());
    }
    let path = PathBuf::from(
        args.get(1)
            .expect("Usage: benchmark <file> [pattern] | generate <file> [MiB]"),
    );
    let pattern = args.get(2).map(String::as_str).unwrap_or("needle");
    let start = Instant::now();
    let data = Dataset::open(&path, &OpenOptionsConfig::default(), &Arc::default())?;
    println!(
        "bytes={} rows={} columns={} index_entries={} index_seconds={:.3} index_MiB_per_second={:.1}",
        data.bytes,
        data.rows,
        data.headers.len(),
        data.checkpoints.len(),
        start.elapsed().as_secs_f64(),
        data.bytes as f64 / 1048576.0 / start.elapsed().as_secs_f64()
    );
    let start = Instant::now();
    let last = data.record(data.rows - 1)?;
    println!(
        "last_row_seek_ms={:.3} last_row_first_field={}",
        start.elapsed().as_secs_f64() * 1000.0,
        String::from_utf8_lossy(&last[0])
    );
    let request = ScanRequest {
        find: Some(FindSpec {
            text: pattern.into(),
            regex: false,
            case_sensitive: false,
            scope: Scope::Table,
        }),
        ..Default::default()
    };
    let result = search::scan(&data, &request, &Arc::default())?;
    println!(
        "hits={} matching_rows={} search_seconds={:.3} search_MiB_per_second={:.1}",
        result.hits,
        result.matched_rows.len(),
        result.elapsed.as_secs_f64(),
        data.bytes as f64 / 1048576.0 / result.elapsed.as_secs_f64()
    );
    if pattern == "needle" {
        ensure!(result.hits == 1, "Expected exactly one planted match");
        ensure!(
            result.hit_page(0, 1)?[0].row == data.rows - 1,
            "Incorrect match position"
        );
    }
    let types = timeline::query::infer_types(&data)?;
    let start = Instant::now();
    let view = timeline::query::execute(
        &data,
        &timeline::query::Query {
            text: pattern.into(),
            ..Default::default()
        },
        &types,
        &Default::default(),
    )?;
    println!(
        "web_query_rows={} web_query_seconds={:.3}",
        view.rows,
        start.elapsed().as_secs_f64()
    );
    ensure!(
        view.rows == result.matched_rows.len(),
        "New and existing engines disagree"
    );
    if pattern == "needle" {
        ensure!(
            view.viewport(&data, 0, 1, &[0])?[0].id == data.rows - 1,
            "Web query missed the final record"
        );
    }
    Ok(())
}
