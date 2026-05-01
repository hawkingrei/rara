use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredExperience {
    text: String,
    created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ExperienceFile {
    experiences: Vec<StoredExperience>,
}

pub struct ExperienceStore {
    path: PathBuf,
    state: Mutex<InnerState>,
}

struct InnerState {
    data: ExperienceFile,
    dirty: bool,
}

impl ExperienceStore {
    pub fn new(dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("experiences.json");
        let data = if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            ExperienceFile::default()
        };
        Ok(Self {
            path,
            state: Mutex::new(InnerState { data, dirty: false }),
        })
    }

    /// Persist an experience atomically.
    pub fn remember(&self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut guard = self.state.lock().expect("experience store poisoned");
        guard.data.experiences.push(StoredExperience {
            text: trimmed.to_string(),
            created_at: now,
        });
        let json = serde_json::to_string_pretty(&guard.data).expect("serialize experiences");
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).expect("write experiences tmp file");
        std::fs::rename(&tmp, &self.path).expect("commit experiences file");
    }

    /// Retrieve experiences matching query keywords.
    /// Results are ranked by match score descending, with recency breaking ties.
    pub fn retrieve(&self, query: &str, limit: usize) -> Vec<String> {
        let keywords = query_keywords(query);
        let guard = self.state.lock().expect("experience store poisoned");
        let mut scored: Vec<(usize, usize, &StoredExperience)> = guard
            .data
            .experiences
            .iter()
            .enumerate()
            .filter_map(|(idx, exp)| {
                let score = match_score(&keywords, &exp.text);
                if score > 0 {
                    Some((idx, score, exp))
                } else {
                    None
                }
            })
            .collect();
        // Sort by score descending, then recency (higher index = more recent) as tiebreaker
        scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
        scored
            .into_iter()
            .take(limit)
            .map(|(_, _, exp)| exp.text.clone())
            .collect()
    }

    /// Number of stored experiences.
    pub fn len(&self) -> usize {
        let guard = self.state.lock().expect("experience store poisoned");
        guard.data.experiences.len()
    }
}

fn query_keywords(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| !w.is_empty() && w.len() >= 2)
        .collect()
}

fn match_score(keywords: &[String], text: &str) -> usize {
    let lower = text.to_lowercase();
    keywords
        .iter()
        .filter(|kw| lower.contains(kw.as_str()))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remember_and_retrieve() {
        let dir = tempfile::tempdir().unwrap();
        let store = ExperienceStore::new(dir.path().to_path_buf()).unwrap();

        store.remember("the user prefers Rust over Python");
        store.remember("rara project uses ratatui for TUI");
        store.remember("vector db is not yet implemented");

        let results = store.retrieve("rust", 5);
        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.contains("Rust")));
    }

    #[test]
    fn keyword_match_scoring() {
        let dir = tempfile::tempdir().unwrap();
        let store = ExperienceStore::new(dir.path().to_path_buf()).unwrap();

        store.remember("apple banana cherry");
        store.remember("apple date elderberry");
        store.remember("fig grape honeydew");

        let results = store.retrieve("apple banana", 5);
        assert!(
            results.iter().any(|r| r.contains("cherry")),
            "should find the apple+banana matching entry"
        );
    }

    #[test]
    fn empty_query_returns_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = ExperienceStore::new(dir.path().to_path_buf()).unwrap();
        store.remember("some text");
        let results = store.retrieve("", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn limit_respected() {
        let dir = tempfile::tempdir().unwrap();
        let store = ExperienceStore::new(dir.path().to_path_buf()).unwrap();
        for i in 0..10 {
            store.remember(&format!("experience number {i}"));
        }
        let results = store.retrieve("experience", 3);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = ExperienceStore::new(dir.path().to_path_buf()).unwrap();
        assert_eq!(store.len(), 0);
        let results = store.retrieve("anything", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn persistence_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();

        {
            let store = ExperienceStore::new(path.clone()).unwrap();
            store.remember("persisted experience one");
            store.remember("persisted experience two");
        }

        let store2 = ExperienceStore::new(path).unwrap();
        assert_eq!(store2.len(), 2);
        let results = store2.retrieve("persisted", 5);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn score_ranking_preferred_over_recency() {
        let dir = tempfile::tempdir().unwrap();
        let store = ExperienceStore::new(dir.path().to_path_buf()).unwrap();

        store.remember("apple banana cherry");
        store.remember("apple cherry");

        let results = store.retrieve("apple banana cherry", 5);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], "apple banana cherry");
        assert_eq!(results[1], "apple cherry");
    }

    #[test]
    fn recency_breaks_score_ties() {
        let dir = tempfile::tempdir().unwrap();
        let store = ExperienceStore::new(dir.path().to_path_buf()).unwrap();

        store.remember("older rust project");
        store.remember("newer rust project");

        let results = store.retrieve("rust", 5);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], "newer rust project");
    }

    #[test]
    fn is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = ExperienceStore::new(dir.path().to_path_buf()).unwrap();
        assert_eq!(store.len(), 0);
        store.remember("test");
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn keyword_splitting() {
        let kw = query_keywords("Rust, Python! and Go");
        assert_eq!(kw, vec!["rust", "python", "and", "go"]);
    }

    #[test]
    fn match_score_counting() {
        let kw = vec!["rust".to_string(), "python".to_string()];
        assert_eq!(match_score(&kw, "I like Rust and Python"), 2);
        assert_eq!(match_score(&kw, "I like Rust"), 1);
        assert_eq!(match_score(&kw, "I like Go"), 0);
    }
}
