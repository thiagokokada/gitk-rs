//! Opt-in real-history accounting; normal tests never scan a user's large repo.
use gitk_rs::{
    git::{CommitId, Repository},
    graph::{Edge, GraphBuilder},
    history::Entry,
};
use std::{
    env,
    mem::size_of,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

#[test]
#[ignore = "set GITK_BENCH_REPOSITORY; scans 200,000 real commits by default"]
fn real_history_allocation_and_cpu() {
    let path = PathBuf::from(env::var_os("GITK_BENCH_REPOSITORY").expect("GITK_BENCH_REPOSITORY"));
    let count: usize = env::var("GITK_BENCH_COMMITS")
        .unwrap_or("200000".into())
        .parse()
        .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let repository = Repository::open(&path, &cancel).unwrap();
    let mut stream = repository.log(cancel).unwrap();
    let mut graph = GraphBuilder::new(200);
    let mut graph_bytes = 0;
    let mut parent_bytes = 0;
    let mut read_time = Duration::ZERO;
    let mut graph_time = Duration::ZERO;
    for loaded in 1..=count {
        let started = Instant::now();
        let commit = stream
            .next_commit()
            .unwrap()
            .expect("history shorter than requested count");
        read_time += started.elapsed();
        let started = Instant::now();
        let row = graph.push(&commit);
        graph_time += started.elapsed();
        graph_bytes += row.edges.len() * size_of::<Edge>();
        parent_bytes += commit.parents.len() * size_of::<CommitId>();
        if loaded % 100000 == 0 || loaded == count {
            println!(
                "loaded={loaded} edge_payload_bytes={graph_bytes} parent_payload_bytes={parent_bytes} entry_size={} read_ms={} graph_ms={}",
                size_of::<Entry>(),
                read_time.as_millis(),
                graph_time.as_millis()
            );
        }
    }
}
