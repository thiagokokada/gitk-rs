//! Ordered, bounded parallel object reads from one revision traversal.
use super::*;
use std::sync::mpsc::{self, SyncSender};

const BATCH_COMMITS: usize = 64;
const BATCH_BYTES: usize = MAX_RECORD_BYTES;

#[cfg(test)]
type Children = Arc<Mutex<Vec<Arc<Mutex<Child>>>>>;

type BatchReceiver = Receiver<Result<CommitBatch>>;

struct CommitBatch {
    commits: Vec<Commit>,
    complete: bool,
}

struct Work {
    records: Vec<u8>,
    count: usize,
    output: SyncSender<Result<CommitBatch>>,
}

enum Ordered {
    Batch(BatchReceiver),
    Finished,
}

struct Worker {
    input: SyncSender<Work>,
    handle: JoinHandle<Result<()>>,
}

pub(super) struct CommitPool {
    #[cfg(test)]
    children: Children,
    order: Option<Receiver<Result<Ordered>>>,
    current: Option<BatchReceiver>,
    chunk: std::vec::IntoIter<Commit>,
    complete: bool,
    done: bool,
    cancel: Cancellation,
    parent: Cancellation,
    stopped: Arc<AtomicBool>,
    bridge: Option<JoinHandle<()>>,
    coordinator: Option<JoinHandle<()>>,
}

impl CommitPool {
    pub(super) fn open(
        root: &Path,
        head: &CommitId,
        parent: Cancellation,
        readers: usize,
    ) -> Result<Self> {
        let cancel = Cancellation::default();
        let stopped = Arc::new(AtomicBool::new(false));
        let bridge = {
            let parent = parent.clone();
            let cancel = cancel.clone();
            let stopped = stopped.clone();
            thread::Builder::new()
                .name("history-cancel".into())
                .spawn(move || {
                    parent.wait(&stopped);
                    cancel.store(true, Ordering::Release);
                })?
        };
        #[cfg(test)]
        let children: Children = Arc::default();
        let (sender, receiver) = mpsc::sync_channel(readers);
        let mut pool = Self {
            #[cfg(test)]
            children: children.clone(),
            order: Some(receiver),
            current: None,
            chunk: Vec::new().into_iter(),
            complete: false,
            done: false,
            cancel: cancel.clone(),
            parent,
            stopped,
            bridge: Some(bridge),
            coordinator: None,
        };
        let root = root.to_owned();
        let head = head.clone();
        pool.coordinator = Some(
            thread::Builder::new()
                .name("history-dispatch".into())
                .spawn(move || {
                    let mut workers = Vec::new();
                    let mut result = dispatch(
                        &root,
                        &head,
                        &cancel,
                        readers,
                        &sender,
                        &mut workers,
                        #[cfg(test)]
                        &children,
                    );
                    if result.is_err() {
                        cancel.store(true, Ordering::Release);
                    }
                    // Close every request channel before joining any worker. Successful
                    // workers close cat-file stdin, read EOF, and check its exit status.
                    let handles: Vec<_> = workers
                        .into_iter()
                        .map(|worker| {
                            drop(worker.input);
                            worker.handle
                        })
                        .collect();
                    for handle in handles {
                        let finished = handle
                            .join()
                            .map_err(|_| anyhow::anyhow!("Git object worker panicked"))
                            .and_then(|r| r);
                        if result.is_ok() && finished.is_err() {
                            result = finished;
                            cancel.store(true, Ordering::Release);
                        }
                    }
                    let terminal = result.map(|()| Ordered::Finished);
                    let _ = sender.send(terminal);
                })?,
        );
        Ok(pool)
    }

    pub(super) fn next_commit(&mut self) -> Result<Option<Commit>> {
        if self.done {
            return Ok(None);
        }
        loop {
            ensure!(
                !self.parent.load(Ordering::Acquire),
                "Git operation cancelled"
            );
            if let Some(commit) = self.chunk.next() {
                return Ok(Some(commit));
            }
            if self.complete {
                self.current = None;
                self.complete = false;
            }
            if self.current.is_none() {
                match self
                    .order
                    .as_ref()
                    .unwrap()
                    .recv()
                    .context("Git history coordinator stopped before EOF")??
                {
                    Ordered::Batch(receiver) => self.current = Some(receiver),
                    Ordered::Finished => {
                        self.done = true;
                        return Ok(None);
                    }
                }
            }
            let batch = self
                .current
                .as_ref()
                .unwrap()
                .recv()
                .context("Git object reader stopped before completing its batch")??;
            self.complete = batch.complete;
            self.chunk = batch.commits.into_iter();
        }
    }
}

impl Drop for CommitPool {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.stopped.store(true, Ordering::Release);
        self.parent.wake();
        // Drop receivers before joining: output may be backpressured while the
        // user pauses prefetch or replaces the stream. This wakes blocked sends.
        self.current.take();
        self.order.take();
        if let Some(coordinator) = self.coordinator.take() {
            let _ = coordinator.join();
        }
        if let Some(bridge) = self.bridge.take() {
            let _ = bridge.join();
        }
    }
}

fn dispatch(
    root: &Path,
    head: &CommitId,
    cancel: &Cancellation,
    readers: usize,
    order: &SyncSender<Result<Ordered>>,
    workers: &mut Vec<Worker>,
    #[cfg(test)] children: &Children,
) -> Result<()> {
    let mut traversal = GitStream::spawn(
        root,
        &["rev-list", "--parents", &head.to_hex()],
        cancel.clone(),
        &[0],
    )?;
    #[cfg(test)]
    children.lock().unwrap().push(traversal.child.clone());
    let mut line = Vec::new();
    let mut pending = false;
    let mut eof = false;
    let mut index = 0;
    while !eof || pending {
        ensure!(!cancel.load(Ordering::Acquire), "Git operation cancelled");
        let mut records = Vec::new();
        let mut count = 0;
        while count < BATCH_COMMITS {
            if !pending {
                match traversal.read_record(b'\n', &mut line)? {
                    None => {
                        eof = true;
                        break;
                    }
                    Some(terminated) => {
                        ensure!(terminated, "Git rev-list ended in an incomplete record")
                    }
                }
            }
            if count > 0 && records.len() + line.len() + 1 > BATCH_BYTES {
                pending = true;
                break;
            }
            records.extend_from_slice(&line);
            records.push(b'\n');
            pending = false;
            count += 1;
        }
        if count == 0 {
            break;
        }
        if index == workers.len() {
            // Start readers lazily. Reader zero can publish the first viewport
            // while subsequent Git processes are starting in the background.
            workers.push(start_worker(
                root,
                cancel.clone(),
                #[cfg(test)]
                children.clone(),
            )?);
        }
        let (output, receiver) = mpsc::sync_channel(1);
        workers[index]
            .input
            .send(Work {
                records,
                count,
                output,
            })
            .map_err(|_| anyhow::anyhow!("Git object worker stopped accepting input"))?;
        order
            .send(Ok(Ordered::Batch(receiver)))
            .map_err(|_| anyhow::anyhow!("Git history receiver closed"))?;
        index = (index + 1) % readers;
    }
    Ok(())
}

fn start_worker(
    root: &Path,
    cancel: Cancellation,
    #[cfg(test)] children: Children,
) -> Result<Worker> {
    let (input, receiver) = mpsc::sync_channel::<Work>(1);
    let root = root.to_owned();
    let handle = thread::Builder::new()
        .name("history-objects".into())
        .spawn(move || {
            let (writer, records) = mpsc::sync_channel(0);
            let mut reader = GitStream::spawn_inner(
                &root,
                &[
                    "cat-file",
                    "--batch=%(objectname) %(objecttype) %(objectsize) %(rest)",
                ],
                cancel.clone(),
                &[0],
                Some(GitInput::Records(records)),
                None,
            )?;
            #[cfg(test)]
            children.lock().unwrap().push(reader.child.clone());
            let mut buffer = Vec::new();
            while let Ok(mut work) = receiver.recv() {
                let result =
                    read_batch(&root, &mut reader, &mut buffer, &writer, &mut work, &cancel);
                if let Err(error) = result {
                    let _ = work.output.send(Err(anyhow::anyhow!("{error:#}")));
                    return Err(error);
                }
            }
            drop(writer);
            ensure!(
                read_raw_commit(&root, &mut reader, &mut buffer)?.is_none(),
                "Git cat-file returned extra objects"
            );
            Ok(())
        })?;
    Ok(Worker { input, handle })
}

fn read_batch(
    root: &Path,
    reader: &mut GitStream,
    buffer: &mut Vec<u8>,
    writer: &SyncSender<Vec<u8>>,
    work: &mut Work,
    cancel: &Cancellation,
) -> Result<()> {
    writer
        .send(std::mem::take(&mut work.records))
        .map_err(|_| anyhow::anyhow!("Git cat-file input closed"))?;
    let mut commits = Vec::with_capacity(work.count);
    let mut bytes = 0;
    for index in 0..work.count {
        ensure!(!cancel.load(Ordering::Acquire), "Git operation cancelled");
        let commit = read_raw_commit(root, reader, buffer)
            .context("read Git cat-file object")?
            .context("Git cat-file ended before its requested objects")?;
        bytes += commit.memory_bytes();
        commits.push(commit);
        let complete = index + 1 == work.count;
        if bytes >= BATCH_BYTES || complete {
            work.output
                .send(Ok(CommitBatch {
                    commits: std::mem::take(&mut commits),
                    complete,
                }))
                .map_err(|_| anyhow::anyhow!("Git object batch receiver closed"))?;
            bytes = 0;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Fixture;
    use std::time::Instant;

    fn children(stream: &CommitStream, expected: usize) -> Vec<Arc<Mutex<Child>>> {
        let CommitSource::Pool(pool) = &stream.source else {
            panic!("expected object pool")
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let children = pool.children.lock().unwrap().clone();
            if children.len() == expected {
                return children;
            }
            assert!(Instant::now() < deadline, "object readers failed to start");
            thread::yield_now();
        }
    }

    fn close(
        stream: CommitStream,
        parent: &Cancellation,
        cancel: bool,
        children: Vec<Arc<Mutex<Child>>>,
    ) {
        if cancel {
            parent.store(true, Ordering::Release);
        }
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            drop(stream);
            sender.send(()).unwrap();
        });
        receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("pool cleanup blocked under backpressure");
        worker.join().unwrap();
        assert_eq!(parent.load(Ordering::Acquire), cancel);
        for child in children {
            assert!(
                child.lock().unwrap().try_wait().unwrap().is_some(),
                "Git child was not reaped"
            );
        }
    }

    #[test]
    fn dropping_or_cancelling_a_paused_pool_reaps_all_children() {
        let fixture = Fixture::new();
        fixture.commit("root");
        fixture.append_history(2000, false, "candidate");
        for readers in [2, 4, 8] {
            for cancel in [false, true] {
                let parent = Cancellation::default();
                let repository = Repository::open(fixture.root(), &parent).unwrap();
                let mut stream = repository
                    .log_with_readers(parent.clone(), readers)
                    .unwrap();
                for _ in 0..BATCH_COMMITS * readers + 1 {
                    assert!(stream.next_commit().unwrap().is_some());
                }
                let children = children(&stream, readers + 1);
                close(stream, &parent, cancel, children);
            }
        }
    }

    #[test]
    fn byte_bounded_output_splits_batches_and_unblocks_on_drop_or_cancel() {
        let fixture = Fixture::new();
        fixture.commit("root");
        fixture.append_history(128, false, &"x".repeat(512 * 1024));
        for cancel in [false, true] {
            let parent = Cancellation::default();
            let repository = Repository::open(fixture.root(), &parent).unwrap();
            let mut stream = repository.log_with_readers(parent.clone(), 2).unwrap();
            assert_eq!(
                stream.next_commit().unwrap().unwrap().message.len(),
                512 * 1024
            );
            let CommitSource::Pool(pool) = &stream.source else {
                panic!("expected object pool")
            };
            assert!(
                pool.chunk.len() < BATCH_COMMITS - 1,
                "a whole large batch was retained in one output chunk"
            );
            assert!(!pool.complete, "large batches must be split by byte budget");
            let children = children(&stream, 3);
            close(stream, &parent, cancel, children);
        }
    }

    #[test]
    fn dropping_reload_pool_does_not_cancel_the_retained_stream() {
        let fixture = Fixture::new();
        fixture.commit("root");
        fixture.append_history(300, false, "retained");
        let parent = Cancellation::default();
        let repository = Repository::open(fixture.root(), &parent).unwrap();
        let mut retained = repository.log_with_readers(parent.clone(), 2).unwrap();
        let first = retained.next_commit().unwrap().unwrap();
        let mut reload = repository.log_with_readers(parent.clone(), 4).unwrap();
        assert_eq!(reload.next_commit().unwrap().unwrap().id, first.id);
        drop(reload);
        assert!(!parent.load(Ordering::Acquire));
        assert_eq!(
            retained.next_commit().unwrap().unwrap().id,
            first.parents[0]
        );
    }

    #[test]
    fn object_reader_failure_is_reported_and_children_are_reaped() {
        let fixture = Fixture::new();
        fixture.commit("root");
        fixture.append_history(4000, false, "candidate");
        let parent = Cancellation::default();
        let repository = Repository::open(fixture.root(), &parent).unwrap();
        let mut stream = repository.log_with_readers(parent.clone(), 2).unwrap();
        for _ in 0..BATCH_COMMITS * 2 {
            stream.next_commit().unwrap().unwrap();
        }
        let children = children(&stream, 3);
        terminate(&mut children[2].lock().unwrap());
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let error = loop {
                match stream.next_commit() {
                    Ok(Some(_)) => (),
                    Ok(None) => panic!("object reader failure was hidden by EOF"),
                    Err(error) => break error,
                }
            };
            drop(stream);
            sender.send(error.to_string()).unwrap();
        });
        let error = receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("object reader failure blocked cleanup");
        worker.join().unwrap();
        assert!(error.contains("Git"), "{error}");
        assert!(!parent.load(Ordering::Acquire));
        for child in children {
            assert!(child.lock().unwrap().try_wait().unwrap().is_some());
        }
    }
}
