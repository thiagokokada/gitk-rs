//! Optional functional probe for a local repository; no GUI and no history cache.
use anyhow::{Result, bail};
use gitk_rs::history::{Generation, HistoryEngine, Request};
use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let repository = args.next().map(PathBuf::from).unwrap_or_else(|| ".".into());
    let wanted = args
        .next()
        .and_then(|s| s.to_str().and_then(|s| s.parse::<usize>().ok()))
        .unwrap_or(1000);
    let started = Instant::now();
    let engine = HistoryEngine::spawn(Request {
        generation: Generation(1),
        id: 0,
        refs_revision: 0,
        repository,
        range: 0..wanted,
        query: String::new(),
        batch: 1000,
        graph_columns: 200,
        anchor: None,
    });
    let handle = engine.handle();
    let mut first = true;
    loop {
        if let Some(snapshot) = handle.take_snapshot() {
            if let Some(error) = snapshot.error {
                bail!(error);
            }
            if first && !snapshot.rows.is_empty() {
                println!("first rows: {:?}", started.elapsed());
                first = false;
            }
            if snapshot.loaded >= wanted || snapshot.eof {
                println!(
                    "loaded={} displayed={} eof={} elapsed={:?}",
                    snapshot.loaded,
                    snapshot.rows.len(),
                    snapshot.eof,
                    started.elapsed()
                );
                if let Some(row) = snapshot.rows.first() {
                    println!("{} {}", row.commit.id.as_str(), row.commit.summary());
                }
                break;
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    drop(engine);
    Ok(())
}
