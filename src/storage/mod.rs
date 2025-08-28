//! Storage abstractions and implementations
//! 
//! This module provides persistent storage for Raft logs, node state,
//! and key-value data with trait-based interfaces for dependency injection.

pub mod log_storage;
pub mod state_storage;
pub mod kv_storage;

pub use log_storage::{LogStorage, FileLogStorage, InMemoryLogStorage};
pub use state_storage::{StateStorage, FileStateStorage, InMemoryStateStorage};
pub use kv_storage::{KVStorage, FileKVStorage, InMemoryKVStorage};

use crate::{NodeId, Term, LogIndex};

/// A log entry in the Raft log
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    /// The term when this entry was created
    pub term: Term,
    /// The index of this entry in the log
    pub index: LogIndex,
    /// The command data
    pub data: Vec<u8>,
}

impl LogEntry {
    /// Create a new log entry
    pub fn new(term: Term, index: LogIndex, data: Vec<u8>) -> Self {
        Self { term, index, data }
    }
}

/// Persistent state that must be maintained across restarts
#[derive(Debug, Clone, PartialEq)]
pub struct PersistentState {
    /// Latest term server has seen
    pub current_term: Term,
    /// Candidate that received vote in current term (or None)
    pub voted_for: Option<NodeId>,
}

impl Default for PersistentState {
    fn default() -> Self {
        Self {
            current_term: 0,
            voted_for: None,
        }
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_entry_creation() {
        let entry = LogEntry::new(1, 5, b"test data".to_vec());
        assert_eq!(entry.term, 1);
        assert_eq!(entry.index, 5);
        assert_eq!(entry.data, b"test data");
    }

    #[test]
    fn test_persistent_state_default() {
        let state = PersistentState::default();
        assert_eq!(state.current_term, 0);
        assert_eq!(state.voted_for, None);
    }
}
