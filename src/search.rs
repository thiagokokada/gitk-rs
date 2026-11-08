//! Parsed commit filters shared by the worker and search feedback UI.
use crate::git::Commit;
use anyhow::{Result, ensure};
use std::{fmt, str::FromStr};

/// Supported qualifiers and their user-facing names share one inventory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Qualifier {
    Author,
    Committer,
    Message,
    Hash,
    Path,
    Before,
    After,
    Merge,
}

impl Qualifier {
    pub const ALL: [Self; 8] = [
        Self::Author,
        Self::Committer,
        Self::Message,
        Self::Hash,
        Self::Path,
        Self::Before,
        Self::After,
        Self::Merge,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Author => "author",
            Self::Committer => "committer",
            Self::Message => "message",
            Self::Hash => "hash",
            Self::Path => "path",
            Self::Before => "before",
            Self::After => "after",
            Self::Merge => "merge",
        }
    }

    pub fn syntax(self) -> String {
        let value = match self {
            Self::Before | Self::After => "YYYY-MM-DD",
            Self::Merge => "true|false",
            Self::Author | Self::Committer | Self::Message | Self::Hash | Self::Path => "text",
        };
        format!("{self}:{value}")
    }

    pub const fn help_description(self) -> &'static str {
        match self {
            Self::Author => "Author name or email, e.g. Alice",
            Self::Committer => "Committer name or email, e.g. Bob",
            Self::Message => "Full commit message substring",
            Self::Hash => "Commit hash substring",
            Self::Path => "Changed path substring; merges compare with their first parent",
            Self::Before => "Author's recorded calendar day strictly before this date",
            Self::After => "Author's recorded calendar day strictly after this date",
            Self::Merge => "Include merges with true; exclude merges with false",
        }
    }

    fn validate(self, value: &str) -> Result<()> {
        ensure!(!value.is_empty(), "{self}: needs a value");
        match self {
            Self::Before | Self::After => {
                ensure!(valid_date(value), "{self}: use a valid YYYY-MM-DD date")
            }
            Self::Merge => ensure!(
                matches!(value, "true" | "false"),
                "{self}: expects true or false"
            ),
            Self::Author | Self::Committer | Self::Message | Self::Hash | Self::Path => (),
        }
        Ok(())
    }
}

impl fmt::Display for Qualifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Qualifier {
    type Err = anyhow::Error;

    fn from_str(name: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|qualifier| qualifier.name().eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Unknown filter {}: ({})",
                    name.to_ascii_lowercase(),
                    Self::ALL.map(Self::name).join(", ")
                )
            })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Query {
    terms: Vec<(Qualifier, String)>,
    text: String,
    pub path: Option<String>,
    pub filters: Vec<String>,
}

impl Query {
    pub fn parse(input: &str) -> Result<Self> {
        ensure!(
            input.len() <= 8192,
            "filter is too long (maximum 8192 bytes)"
        );
        let mut tokens = Vec::new();
        let mut token = String::new();
        let mut quoted = false;
        for ch in input.chars() {
            match ch {
                '"' => quoted = !quoted,
                ch if ch.is_whitespace() && !quoted => {
                    if !token.is_empty() {
                        tokens.push(std::mem::take(&mut token));
                    }
                }
                _ => token.push(ch),
            }
        }
        ensure!(!quoted, "Close the quoted search value with a double quote");
        if !token.is_empty() {
            tokens.push(token);
        }
        let mut result = Self::default();
        let mut text = Vec::new();
        for token in tokens {
            let (key, value) = token.split_once(':').unwrap_or(("", &token));
            if key.is_empty() {
                text.push(value.to_lowercase());
                continue;
            }
            let qualifier: Qualifier = key.parse()?;
            qualifier.validate(value)?;
            if qualifier == Qualifier::Path {
                ensure!(
                    result.path.is_none(),
                    "Use one {qualifier}: filter per search"
                );
                result.path = Some(value.to_owned());
            } else {
                result.terms.push((qualifier, value.to_lowercase()));
            }
            result.filters.push(format!("{qualifier}:{value}"));
        }
        result.text = text.join(" ");
        Ok(result)
    }

    /// Paths are resolved separately by one cancellable Git traversal.
    pub fn matches(&self, commit: &Commit) -> bool {
        let metadata_matches = self.terms.iter().all(|(qualifier, value)| {
            match qualifier {
                Qualifier::Author => {
                    contains(&commit.author.name, value) || contains(&commit.author.email, value)
                }
                Qualifier::Committer => {
                    contains(&commit.committer.name, value)
                        || contains(&commit.committer.email, value)
                }
                Qualifier::Message => contains(&commit.message, value),
                Qualifier::Hash => commit.id.to_hex().contains(value),
                // ISO dates compare lexically. Use the author's recorded calendar day,
                // retaining its timezone rather than silently applying the local timezone.
                Qualifier::Before => commit
                    .author
                    .time
                    .get(..10)
                    .is_some_and(|day| day < value.as_str()),
                Qualifier::After => commit
                    .author
                    .time
                    .get(..10)
                    .is_some_and(|day| day > value.as_str()),
                Qualifier::Merge => (commit.parents.len() > 1) == (value == "true"),
                Qualifier::Path => true, // Resolved by the history worker's diff stream.
            }
        });
        metadata_matches && (self.text.is_empty() || commit.searchable_text().contains(&self.text))
    }
}

fn contains(text: &str, needle: &str) -> bool {
    text.to_lowercase().contains(needle)
}

fn valid_date(value: &str) -> bool {
    let b = value.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || b.iter()
            .enumerate()
            .any(|(i, b)| i != 4 && i != 7 && !b.is_ascii_digit())
    {
        return false;
    }
    let year: u32 = value[..4].parse().unwrap();
    let month: usize = value[5..7].parse().unwrap();
    let day: u32 = value[8..].parse().unwrap();
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    year > 0 && (1..=12).contains(&month) && day > 0 && day <= days[month - 1]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn advertised_qualifiers_accept_case_insensitive_names_and_canonicalize_badges() {
        for qualifier in Qualifier::ALL {
            let value = match qualifier {
                Qualifier::Before | Qualifier::After => "2026-01-01",
                Qualifier::Merge => "false",
                Qualifier::Author
                | Qualifier::Committer
                | Qualifier::Message
                | Qualifier::Hash
                | Qualifier::Path => "Alice",
            };
            let query = Query::parse(&format!(
                "{}:{value}",
                qualifier.name().to_ascii_uppercase()
            ))
            .unwrap();
            assert_eq!(query.filters, [format!("{qualifier}:{value}")]);
        }
    }

    #[test]
    fn unknown_filters_are_reported_before_missing_values() {
        for input in ["abc:", "abc:value", "ABC:"] {
            let error = Query::parse(input).unwrap_err().to_string();
            assert!(error.starts_with("Unknown filter abc:"), "{input}: {error}");
        }
        for key in Qualifier::ALL {
            let error = Query::parse(&format!("{key}:")).unwrap_err().to_string();
            assert_eq!(error, format!("{key}: needs a value"));
        }
    }

    #[test]
    fn validates_and_explains_filters() {
        let query =
            Query::parse("fix author:\"Alice Smith\" path:ci.yml after:2024-02-29 merge:false")
                .unwrap();
        assert_eq!(query.path.as_deref(), Some("ci.yml"));
        assert_eq!(query.filters.len(), 4);
        for bad in [
            "author:",
            "before:2025-02-29",
            "after:2026-13-01",
            "before:2026-00-01",
            "after:0000-01-01",
            "merge:yes",
            "from:abc",
            "path:a path:b",
            "author:\"abc",
        ] {
            assert!(Query::parse(bad).is_err(), "{bad}");
        }
        assert!(Query::parse("before:2000-02-29").is_ok());
    }
}
