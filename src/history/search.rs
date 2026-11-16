//! Query transitions, bounded diff-search workers and incremental prefix filtering.
use super::*;

pub(super) struct PrefixSearch {
    range: Range<usize>,
    suffix_matches: Vec<usize>,
    suffix_candidates: Vec<(CommitId, usize)>,
}

#[derive(Default)]
pub(super) struct HistorySearch {
    pub(super) query: String,
    regex: bool,
    pub(super) parsed: CompiledQuery,
    pub(super) query_error: Option<String>,
    pub(super) paths: Option<DiffSearch>,
    pub(super) path_pending: Vec<CommitId>,
    pub(super) candidates: HashMap<CommitId, usize>,
    pub(super) matches: Vec<usize>,
    pub(super) scanned: usize,
    pub(super) prefix_search: Option<PrefixSearch>,
}

pub(super) enum SearchProgress {
    Ready { progressed: bool },
    Pending,
    Failed,
}

impl HistorySearch {
    fn clear_progress(&mut self) {
        self.paths = None;
        self.path_pending.clear();
        self.candidates.clear();
        self.matches.clear();
        self.scanned = 0;
        self.prefix_search = None;
    }

    pub(super) fn discard_prefix(&mut self) {
        if self.prefix_search.is_some() {
            // A superseded reconciliation must rescan the retained history.
            self.query.clear();
            self.clear_progress();
        }
    }

    pub(super) fn update_query(
        &mut self,
        request: &Request,
        repository: &Repository,
        workers: usize,
    ) {
        let query = request.query.trim().to_owned();
        if self.query != query || self.regex != request.regex {
            self.clear_progress();
            self.query = query;
            self.regex = request.regex;
            self.query_error = None;
            match Query::parse_with_mode(&self.query, request.regex)
                .and_then(|query| query.compile(request.regex))
            {
                Ok(parsed) => {
                    self.paths = DiffSearch::start(repository, &parsed, workers);
                    self.parsed = parsed;
                }
                Err(error) => self.query_error = Some(error.to_string()),
            }
        }
    }

    fn restart_paths(&mut self, repository: &Repository, workers: usize) {
        // Reap old workers before starting another pool.
        self.paths = None;
        self.paths = DiffSearch::start(repository, &self.parsed, workers);
    }

    pub(super) fn prepend(&mut self, offset: usize, repository: &Repository, workers: usize) {
        for ordinal in &mut self.matches {
            *ordinal += offset;
        }
        for ordinal in self.candidates.values_mut() {
            *ordinal += offset;
        }
        self.scanned += offset;

        if !self.query.is_empty() {
            // Filter the prefix separately before joining the known suffix.
            // This avoids both a delta-sized candidate queue and repeated
            // insertion of prefix matches at the front of a large suffix.
            self.restart_paths(repository, workers);
            let mut suffix_candidates: Vec<_> = self.candidates.drain().collect();
            suffix_candidates.sort_unstable_by_key(|(_, ordinal)| *ordinal);
            self.path_pending.clear();
            self.prefix_search = Some(PrefixSearch {
                range: 0..offset,
                suffix_matches: std::mem::take(&mut self.matches),
                suffix_candidates,
            });
        }
    }

    pub(super) fn consider(&mut self, commit: &Commit, ordinal: usize) {
        if !self.query.is_empty() && self.parsed.matches(commit) {
            if self.paths.is_some() {
                self.candidates.insert(commit.id.clone(), ordinal);
                self.path_pending.push(commit.id.clone());
            } else {
                self.matches.push(ordinal);
            }
        }
    }

    pub(super) fn step(
        &mut self,
        entries: &[Arc<Entry>],
        repository: &Repository,
        workers: usize,
    ) -> Result<SearchProgress> {
        let mut progressed = false;
        if let Some(paths) = &mut self.paths {
            for _ in 0..256 {
                match paths.try_recv() {
                    Ok(Ok((id, matched))) => {
                        progressed = true;
                        let ordinal = self
                            .candidates
                            .remove(&id)
                            .ok_or_else(|| anyhow::anyhow!("Unexpected diff-search commit"))?;
                        if matched {
                            // A reload can queue new prefix candidates before
                            // the remaining suffix has finished filtering.
                            let position = self.matches.partition_point(|i| *i < ordinal);
                            self.matches.insert(position, ordinal);
                        }
                    }
                    Ok(Err(error)) => {
                        self.query_error = Some(format!("Diff search failed: {error:#}"));
                        paths.done = true;
                        return Ok(SearchProgress::Failed);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        paths.done = true;
                        ensure!(
                            self.candidates.is_empty(),
                            "Diff search ended before all commits were checked"
                        );
                        break;
                    }
                }
            }
        }
        if let Some(paths) = &mut self.paths {
            if !self.path_pending.is_empty() {
                let batch = std::mem::take(&mut self.path_pending);
                let pending = batch.len();
                match paths.try_send(batch) {
                    Ok(()) => progressed = true,
                    Err(TrySendError::Full(batch)) => {
                        progressed |= batch.len() < pending;
                        self.path_pending = batch;
                        if !progressed {
                            thread::sleep(Duration::from_millis(5));
                        }
                        return Ok(SearchProgress::Pending);
                    }
                    Err(TrySendError::Disconnected(_)) => {
                        // Another worker may still be waiting for more input. Do
                        // not wait for ordered draining to reach a failed worker.
                        self.query_error = Some("Diff search failed: worker input closed".into());
                        return Ok(SearchProgress::Failed);
                    }
                }
            }
            if self.candidates.len() >= 4096 {
                if !progressed {
                    thread::sleep(Duration::from_millis(5));
                }
                return Ok(SearchProgress::Pending);
            }
        }
        if self
            .prefix_search
            .as_ref()
            .is_some_and(|prefix| prefix.range.is_empty())
        {
            if self.path_pending.is_empty()
                && let Some(paths) = &mut self.paths
            {
                // diff-tree can buffer the last decision until stdin closes.
                paths.close();
            }
            if !self.candidates.is_empty() || !self.path_pending.is_empty() {
                if !progressed {
                    thread::sleep(Duration::from_millis(5));
                }
                return Ok(SearchProgress::Pending);
            }
            let mut prefix = self.prefix_search.take().unwrap();
            self.restart_paths(repository, workers);
            self.matches.append(&mut prefix.suffix_matches);
            for (id, ordinal) in prefix.suffix_candidates {
                self.path_pending.push(id.clone());
                self.candidates.insert(id, ordinal);
            }
            if !self.path_pending.is_empty() {
                return Ok(SearchProgress::Pending);
            }
        }
        let scan = self
            .prefix_search
            .as_ref()
            .map(|prefix| prefix.range.clone())
            .unwrap_or(self.scanned..entries.len());
        if !self.query.is_empty() && !scan.is_empty() {
            let end = (scan.start + 1000).min(scan.end);
            for (index, entry) in entries.iter().enumerate().take(end).skip(scan.start) {
                self.consider(&entry.commit, index);
            }
            if let Some(prefix) = &mut self.prefix_search {
                prefix.range.start = end;
            } else {
                self.scanned = end;
            }
            return Ok(SearchProgress::Pending);
        }
        Ok(SearchProgress::Ready { progressed })
    }
}
