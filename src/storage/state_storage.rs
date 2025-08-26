//! State storage implementations for Raft persistent state
//! 
//! This module provides persistent storage for Raft node state including
//! current term and voted_for information.

use crate::{Result, Error, NodeId, Term};
use super::PersistentState;

/// Trait for persistent state storage
pub trait StateStorage: Send + Sync {
    /// Save the current term
    fn save_current_term(&mut self, term: Term) -> Result<()>;
    
    /// Get the current term
    fn get_current_term(&self) -> Term;
    
    /// Save the candidate voted for in current term
    fn save_voted_for(&mut self, candidate: Option<NodeId>) -> Result<()>;
    
    /// Get the candidate voted for in current term
    fn get_voted_for(&self) -> Option<NodeId>;
    
    /// Save complete persistent state
    fn save_state(&mut self, state: &PersistentState) -> Result<()>;
    
    /// Load complete persistent state
    fn load_state(&self) -> Result<PersistentState>;
}
use std::fs::{File, OpenOptions};
use std::io::{Read, Write, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// File-based state storage implementation
pub struct FileStateStorage {
    /// Path to the state file
    file_path: PathBuf,
    /// Current persistent state
    state: PersistentState,
    /// File handle for reading/writing
    file: Arc<Mutex<File>>,
}

impl FileStateStorage {
    /// Create a new file-based state storage
    pub fn new(file_path: PathBuf) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&file_path)?;
        
        let mut storage = Self {
            file_path: file_path.clone(),
            state: PersistentState::default(),
            file: Arc::new(Mutex::new(file)),
        };
        
        // Load existing state from file
        storage.load_from_file()?;
        
        Ok(storage)
    }
    
    /// Load state from the file
    fn load_from_file(&mut self) -> Result<()> {
        let mut file = self.file.lock().unwrap();
        file.seek(SeekFrom::Start(0))?;
        
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        
        if contents.trim().is_empty() {
            // File is empty, use default state
            return Ok(());
        }
        
        self.state = self.deserialize_state(&contents)?;
        Ok(())
    }
    
    /// Save state to the file
    fn save_to_file(&self) -> Result<()> {
        let serialized = self.serialize_state(&self.state);
        let mut file = self.file.lock().unwrap();
        
        // Truncate and write from beginning
        file.seek(SeekFrom::Start(0))?;
        file.set_len(0)?;
        file.write_all(serialized.as_bytes())?;
        file.flush()?;
        
        Ok(())
    }
    
    /// Serialize persistent state to string
    fn serialize_state(&self, state: &PersistentState) -> String {
        // Simple format: term:voted_for_option
        // voted_for_option is either "none" or the node_id as string
        let voted_for_str = match state.voted_for {
            Some(node_id) => node_id.to_string(),
            None => "none".to_string(),
        };
        format!("{}:{}", state.current_term, voted_for_str)
    }
    
    /// Deserialize persistent state from string
    fn deserialize_state(&self, content: &str) -> Result<PersistentState> {
        let parts: Vec<&str> = content.trim().split(':').collect();
        if parts.len() != 2 {
            return Err(Error::Serialization("Invalid state format".to_string()));
        }
        
        let current_term = parts[0].parse::<Term>()
            .map_err(|e| Error::Serialization(format!("Invalid term: {}", e)))?;
        
        let voted_for = if parts[1] == "none" {
            None
        } else {
            Some(parts[1].parse::<NodeId>()
                .map_err(|e| Error::Serialization(format!("Invalid node_id: {}", e)))?)
        };
        
        Ok(PersistentState {
            current_term,
            voted_for,
        })
    }
}

impl StateStorage for FileStateStorage {
    fn save_current_term(&mut self, term: Term) -> Result<()> {
        let old_term = self.state.current_term;
        if old_term != term {
            log::info!("Updating current term from {} to {} (file: {:?})", old_term, term, self.file_path);
            self.state.current_term = term;
            match self.save_to_file() {
                Ok(()) => {
                    log::debug!("Successfully saved current term {} to persistent storage", term);
                    Ok(())
                }
                Err(e) => {
                    log::error!("Failed to save current term {} to file {:?}: {}", term, self.file_path, e);
                    Err(e)
                }
            }
        } else {
            log::trace!("Current term {} unchanged, skipping save", term);
            Ok(())
        }
    }
    
    fn get_current_term(&self) -> Term {
        log::trace!("Retrieved current term: {}", self.state.current_term);
        self.state.current_term
    }
    
    fn save_voted_for(&mut self, candidate: Option<NodeId>) -> Result<()> {
        let old_vote = self.state.voted_for;
        if old_vote != candidate {
            match candidate {
                Some(node_id) => log::info!("Saving vote for candidate {} in term {} (was: {:?})", 
                                           node_id, self.state.current_term, old_vote),
                None => log::info!("Clearing vote in term {} (was: {:?})", 
                                 self.state.current_term, old_vote),
            }
            self.state.voted_for = candidate;
            match self.save_to_file() {
                Ok(()) => {
                    log::debug!("Successfully saved vote {:?} to persistent storage", candidate);
                    Ok(())
                }
                Err(e) => {
                    log::error!("Failed to save vote {:?} to file {:?}: {}", candidate, self.file_path, e);
                    Err(e)
                }
            }
        } else {
            log::trace!("Vote {:?} unchanged, skipping save", candidate);
            Ok(())
        }
    }
    
    fn get_voted_for(&self) -> Option<NodeId> {
        log::trace!("Retrieved voted_for: {:?}", self.state.voted_for);
        self.state.voted_for
    }
    
    fn save_state(&mut self, state: &PersistentState) -> Result<()> {
        log::info!("Saving complete persistent state: term={}, voted_for={:?}", 
                  state.current_term, state.voted_for);
        let old_state = self.state.clone();
        self.state = state.clone();
        match self.save_to_file() {
            Ok(()) => {
                log::debug!("Successfully saved complete state to persistent storage");
                Ok(())
            }
            Err(e) => {
                log::error!("Failed to save complete state to file {:?}: {}", self.file_path, e);
                // Restore old state on failure
                self.state = old_state;
                Err(e)
            }
        }
    }
    
    fn load_state(&self) -> Result<PersistentState> {
        log::debug!("Loading persistent state: term={}, voted_for={:?}", 
                   self.state.current_term, self.state.voted_for);
        Ok(self.state.clone())
    }
}

/// In-memory state storage for testing
#[derive(Debug, Clone)]
pub struct InMemoryStateStorage {
    state: PersistentState,
}

impl InMemoryStateStorage {
    /// Create a new in-memory state storage
    pub fn new() -> Self {
        Self {
            state: PersistentState::default(),
        }
    }
    
    /// Create with initial state
    pub fn with_state(state: PersistentState) -> Self {
        Self { state }
    }
}

impl Default for InMemoryStateStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl StateStorage for InMemoryStateStorage {
    fn save_current_term(&mut self, term: Term) -> Result<()> {
        let old_term = self.state.current_term;
        if old_term != term {
            log::debug!("In-memory storage: updating current term from {} to {}", old_term, term);
            self.state.current_term = term;
        } else {
            log::trace!("In-memory storage: current term {} unchanged", term);
        }
        Ok(())
    }
    
    fn get_current_term(&self) -> Term {
        log::trace!("In-memory storage: retrieved current term: {}", self.state.current_term);
        self.state.current_term
    }
    
    fn save_voted_for(&mut self, candidate: Option<NodeId>) -> Result<()> {
        let old_vote = self.state.voted_for;
        if old_vote != candidate {
            log::debug!("In-memory storage: updating vote from {:?} to {:?} in term {}", 
                       old_vote, candidate, self.state.current_term);
            self.state.voted_for = candidate;
        } else {
            log::trace!("In-memory storage: vote {:?} unchanged", candidate);
        }
        Ok(())
    }
    
    fn get_voted_for(&self) -> Option<NodeId> {
        log::trace!("In-memory storage: retrieved voted_for: {:?}", self.state.voted_for);
        self.state.voted_for
    }
    
    fn save_state(&mut self, state: &PersistentState) -> Result<()> {
        log::debug!("In-memory storage: saving complete state: term={}, voted_for={:?}", 
                   state.current_term, state.voted_for);
        self.state = state.clone();
        Ok(())
    }
    
    fn load_state(&self) -> Result<PersistentState> {
        log::debug!("In-memory storage: loading state: term={}, voted_for={:?}", 
                   self.state.current_term, self.state.voted_for);
        Ok(self.state.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_in_memory_state_storage() {
        let mut storage = InMemoryStateStorage::new();
        
        assert_eq!(storage.get_current_term(), 0);
        assert_eq!(storage.get_voted_for(), None);
        
        storage.save_current_term(5).unwrap();
        assert_eq!(storage.get_current_term(), 5);
        
        storage.save_voted_for(Some(42)).unwrap();
        assert_eq!(storage.get_voted_for(), Some(42));
        
        let state = PersistentState {
            current_term: 10,
            voted_for: Some(99),
        };
        storage.save_state(&state).unwrap();
        
        let loaded_state = storage.load_state().unwrap();
        assert_eq!(loaded_state.current_term, 10);
        assert_eq!(loaded_state.voted_for, Some(99));
    }

    #[test]
    fn test_state_serialization() {
        // Test state persistence through save/load operations
        let mut storage = InMemoryStateStorage::new();
        
        let state1 = PersistentState {
            current_term: 42,
            voted_for: Some(123),
        };
        storage.save_state(&state1).unwrap();
        let loaded_state1 = storage.load_state().unwrap();
        assert_eq!(state1, loaded_state1);
        
        let state2 = PersistentState {
            current_term: 99,
            voted_for: None,
        };
        storage.save_state(&state2).unwrap();
        let loaded_state2 = storage.load_state().unwrap();
        assert_eq!(state2, loaded_state2);
    }

    #[test]
    fn test_file_state_storage() {
        let temp_dir = tempdir().unwrap();
        let file_path = temp_dir.path().join("state.txt");
        
        // Create storage and save some state
        {
            let mut storage = FileStateStorage::new(file_path.clone()).unwrap();
            storage.save_current_term(15).unwrap();
            storage.save_voted_for(Some(77)).unwrap();
        }
        
        // Create new storage instance and verify state persisted
        {
            let storage = FileStateStorage::new(file_path).unwrap();
            assert_eq!(storage.get_current_term(), 15);
            assert_eq!(storage.get_voted_for(), Some(77));
        }
    }

    #[test]
    fn test_state_serialization_edge_cases() {
        let mut storage = InMemoryStateStorage::new();
        
        // Test with zero term
        let state = PersistentState {
            current_term: 0,
            voted_for: None,
        };
        storage.save_state(&state).unwrap();
        let loaded_state = storage.load_state().unwrap();
        assert_eq!(state, loaded_state);
        
        // Test with large numbers
        let state = PersistentState {
            current_term: u64::MAX,
            voted_for: Some(u64::MAX),
        };
        storage.save_state(&state).unwrap();
        let loaded_state = storage.load_state().unwrap();
        assert_eq!(state, loaded_state);
    }
}
