//! Requested scopes remain separate from the object IDs used by a traversal.
use crate::git::{Cancellation, CommitId, Repository, run_bytes};
use anyhow::{Context, Result, ensure};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum HistoryScope {
    #[default]
    CurrentHead,
    AllBranches,
    Ref(String),
    Range {
        base: String,
        target: String,
    },
}

impl HistoryScope {
    pub fn label(&self) -> String {
        match self {
            Self::CurrentHead => "Current HEAD".into(),
            Self::AllBranches => "All branches (local + remote)".into(),
            Self::Ref(name) => name.clone(),
            Self::Range { base, target } => format!("{base}..{target}"),
        }
    }

    pub(crate) fn resolve(
        &self,
        repository: &Repository,
        cancel: &Cancellation,
    ) -> Result<Vec<String>> {
        match self {
            Self::CurrentHead => Ok(repository.head.iter().map(CommitId::to_hex).collect()),
            Self::Ref(name) => Ok(vec![resolve_commit(repository, name, cancel)?.to_hex()]),
            Self::Range { base, target } => Ok(vec![format!(
                "{}..{}",
                resolve_commit(repository, base, cancel)?.to_hex(),
                resolve_commit(repository, target, cancel)?.to_hex()
            )]),
            Self::AllBranches => {
                let bytes = run_bytes(
                    &repository.root,
                    &[
                        "for-each-ref",
                        "--format=%(objectname)",
                        "refs/heads/",
                        "refs/remotes/",
                    ],
                    cancel,
                    &[0],
                )?;
                let mut tips: Vec<_> = std::str::from_utf8(&bytes)?
                    .lines()
                    .map(|line| CommitId::parse(line).map(|id| id.to_hex()))
                    .collect::<Result<_>>()?;
                tips.extend(repository.head.iter().map(CommitId::to_hex));
                tips.sort();
                tips.dedup();
                Ok(tips)
            }
        }
    }
}

pub fn resolve_commit(
    repository: &Repository,
    revision: &str,
    cancel: &Cancellation,
) -> Result<CommitId> {
    ensure!(!revision.is_empty(), "Enter a revision");
    let expression = format!("{revision}^{{commit}}");
    let bytes = run_bytes(
        &repository.root,
        &["rev-parse", "--verify", "--end-of-options", &expression],
        cancel,
        &[0],
    )
    .with_context(|| {
        format!("Revision {revision:?} is unavailable or does not refer to a commit")
    })?;
    CommitId::parse(std::str::from_utf8(&bytes)?.trim())
}
