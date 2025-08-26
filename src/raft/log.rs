//! Raft log implementation
//! 
//! This module provides the Raft log abstraction and operations
//! for managing log entries in the consensus algorithm.

use crate::{Result, Term, LogIndex};
use crate::storage::LogEntry;
use crate::storage::log_storage::LogStorage;

/// Raft log abstraction
pub struct RaftLog {
    /// Underlying log storage
    storage: Box<dyn LogStorage>,
}

impl RaftLog {
    /// Create a new Raft log with the given storage
    pub fn new(storage: Box<dyn LogStorage>) -> Self {
        Self { storage }
    }
    
    /// Append new entries to the log
    pub fn append_entries(&mut self, entries: Vec<LogEntry>) -> Result<()> {
        self.storage.append_entries(entries)
    }
    
    /// Get a specific log entry by index
    pub fn get_entry(&self, index: LogIndex) -> Option<LogEntry> {
        self.storage.get_entry(index)
    }
    
    /// Get the index of the last log entry
    pub fn last_index(&self) -> LogIndex {
        self.storage.get_last_index()
    }
    
    /// Get the term of the last log entry
    pub fn last_term(&self) -> Term {
        self.storage.get_last_term()
    }
    
    /// Get entries starting from index (inclusive)
    pub fn get_entries_from(&self, index: LogIndex) -> Vec<LogEntry> {
        self.storage.get_entries_from(index)
    }
    
    /// Truncate log from index (inclusive) onwards
    pub fn truncate_from(&mut self, index: LogIndex) -> Result<()> {
        self.storage.truncate_from(index)
    }
    
    /// Check if the log contains an entry at the given index with the given term
    pub fn contains_entry(&self, index: LogIndex, term: Term) -> bool {
        if let Some(entry) = self.get_entry(index) {
            entry.term == term
        } else {
            false
        }
    }
    
    /// Get the term of the entry at the given index
    pub fn term_at(&self, index: LogIndex) -> Option<Term> {
        self.get_entry(index).map(|entry| entry.term)
    }
    
    /// Check if this log is more up-to-date than another log
    /// Returns true if this log is more up-to-date
    pub fn is_more_up_to_date_than(&self, other_last_index: LogIndex, other_last_term: Term) -> bool {
        let my_last_term = self.last_term();
        let my_last_index = self.last_index();
        
        // First compare terms
        if my_last_term != other_last_term {
            return my_last_term > other_last_term;
        }
        
        // If terms are equal, compare indices
        my_last_index >= other_last_index
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{InMemoryLogStorage, LogEntry};

    #[test]
    fn test_raft_log_creation() {
        let storage = Box::new(InMemoryLogStorage::new());
        let log = RaftLog::new(storage);
        
        assert_eq!(log.last_index(), 0);
        assert_eq!(log.last_term(), 0);
    }

    #[test]
    fn test_raft_log_operations() {
        let storage = Box::new(InMemoryLogStorage::new());
        let mut log = RaftLog::new(storage);
        
        // Test appending entries
        let entries = vec![
            LogEntry::new(1, 1, b"entry1".to_vec()),
            LogEntry::new(1, 2, b"entry2".to_vec()),
            LogEntry::new(2, 3, b"entry3".to_vec()),
        ];
        
        log.append_entries(entries).unwrap();
        
        assert_eq!(log.last_index(), 3);
        assert_eq!(log.last_term(), 2);
        
        // Test getting entries
        let entry1 = log.get_entry(1).unwrap();
        assert_eq!(entry1.term, 1);
        assert_eq!(entry1.data, b"entry1");
        
        let entry3 = log.get_entry(3).unwrap();
        assert_eq!(entry3.term, 2);
        assert_eq!(entry3.data, b"entry3");
        
        // Test getting entries from index
        let entries_from_2 = log.get_entries_from(2);
        assert_eq!(entries_from_2.len(), 2);
        assert_eq!(entries_from_2[0].index, 2);
        assert_eq!(entries_from_2[1].index, 3);
        
        // Test contains entry
        assert!(log.contains_entry(1, 1));
        assert!(log.contains_entry(3, 2));
        assert!(!log.contains_entry(1, 2)); // Wrong term
        assert!(!log.contains_entry(4, 2)); // Index doesn't exist
        
        // Test term_at
        assert_eq!(log.term_at(1), Some(1));
        assert_eq!(log.term_at(3), Some(2));
        assert_eq!(log.term_at(4), None);
        
        // Test truncation
        log.truncate_from(2).unwrap();
        assert_eq!(log.last_index(), 1);
        assert_eq!(log.last_term(), 1);
        assert!(log.get_entry(2).is_none());
        assert!(log.get_entry(3).is_none());
    }

    #[test]
    fn test_log_up_to_date_comparison() {
        let storage = Box::new(InMemoryLogStorage::new());
        let mut log = RaftLog::new(storage);
        
        // Add some entries
        let entries = vec![
            LogEntry::new(1, 1, b"entry1".to_vec()),
            LogEntry::new(2, 2, b"entry2".to_vec()),
        ];
        log.append_entries(entries).unwrap();
        
        // Test comparisons
        // Same term, same index -> this log is more up-to-date (>=)
        assert!(log.is_more_up_to_date_than(2, 2));
        
        // Same term, lower index -> this log is more up-to-date
        assert!(log.is_more_up_to_date_than(1, 2));
        
        // Same term, higher index -> this log is not more up-to-date
        assert!(!log.is_more_up_to_date_than(3, 2));
        
        // Higher term -> this log is more up-to-date
        assert!(log.is_more_up_to_date_than(10, 1));
        
        // Lower term -> this log is not more up-to-date
        assert!(!log.is_more_up_to_date_than(1, 3));
    }
}
