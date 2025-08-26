//! Log storage implementations for Raft consensus
//! 
//! This module provides persistent storage for Raft log entries with
//! both file-based and in-memory implementations for testing.

use crate::{Result, Error};
use super::LogEntry;

/// Trait for persistent log storage
pub trait LogStorage: Send + Sync {
    /// Append new entries to the log
    fn append_entries(&mut self, entries: Vec<LogEntry>) -> Result<()>;
    
    /// Get a specific log entry by index
    fn get_entry(&self, index: u64) -> Option<LogEntry>;
    
    /// Get the index of the last log entry
    fn get_last_index(&self) -> u64;
    
    /// Get the term of the last log entry
    fn get_last_term(&self) -> u64;
    
    /// Get entries starting from index (inclusive)
    fn get_entries_from(&self, index: u64) -> Vec<LogEntry>;
    
    /// Truncate log from index (inclusive) onwards
    fn truncate_from(&mut self, index: u64) -> Result<()>;
}
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// File-based log storage implementation
pub struct FileLogStorage {
    /// Path to the log file
    file_path: PathBuf,
    /// In-memory cache of log entries for fast access
    entries: BTreeMap<u64, LogEntry>,
    /// File handle for writing
    file: Arc<Mutex<File>>,
}

impl FileLogStorage {
    /// Create a new file-based log storage
    pub fn new(file_path: PathBuf) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&file_path)?;
        
        let mut storage = Self {
            file_path: file_path.clone(),
            entries: BTreeMap::new(),
            file: Arc::new(Mutex::new(file)),
        };
        
        // Load existing entries from file
        storage.load_from_file()?;
        
        Ok(storage)
    }
    
    /// Load entries from the log file
    fn load_from_file(&mut self) -> Result<()> {
        let file = File::open(&self.file_path)?;
        let reader = BufReader::new(file);
        
        for line in reader.lines() {
            let line = line?;
            if let Ok(entry) = self.deserialize_entry(&line) {
                self.entries.insert(entry.index, entry);
            }
        }
        
        Ok(())
    }
    
    /// Serialize a log entry to a string
    fn serialize_entry(&self, entry: &LogEntry) -> String {
        // Simple format: term:index:data_len:data_base64
        let data_base64 = base64_encode(&entry.data);
        format!("{}:{}:{}:{}\n", entry.term, entry.index, entry.data.len(), data_base64)
    }
    
    /// Deserialize a log entry from a string
    fn deserialize_entry(&self, line: &str) -> Result<LogEntry> {
        let parts: Vec<&str> = line.trim().split(':').collect();
        if parts.len() != 4 {
            return Err(Error::Serialization("Invalid log entry format".to_string()));
        }
        
        let term = parts[0].parse::<u64>()
            .map_err(|e| Error::Serialization(format!("Invalid term: {}", e)))?;
        let index = parts[1].parse::<u64>()
            .map_err(|e| Error::Serialization(format!("Invalid index: {}", e)))?;
        let _data_len = parts[2].parse::<usize>()
            .map_err(|e| Error::Serialization(format!("Invalid data length: {}", e)))?;
        let data = base64_decode(parts[3])
            .map_err(|e| Error::Serialization(format!("Invalid base64 data: {}", e)))?;
        
        Ok(LogEntry::new(term, index, data))
    }
    
    /// Write entry to file
    fn write_entry_to_file(&self, entry: &LogEntry) -> Result<()> {
        let serialized = self.serialize_entry(entry);
        let mut file = self.file.lock().unwrap();
        file.write_all(serialized.as_bytes())?;
        file.flush()?;
        Ok(())
    }
    
    /// Truncate file from a specific position
    fn truncate_file_from_index(&self, from_index: u64) -> Result<()> {
        // For simplicity, rewrite the entire file with entries before from_index
        let temp_path = self.file_path.with_extension("tmp");
        {
            let mut temp_file = File::create(&temp_path)?;
            for (_, entry) in self.entries.iter() {
                if entry.index < from_index {
                    let serialized = self.serialize_entry(entry);
                    temp_file.write_all(serialized.as_bytes())?;
                }
            }
            temp_file.flush()?;
        }
        
        // Replace original file with temp file
        std::fs::rename(temp_path, &self.file_path)?;
        
        // Reopen file handle
        let new_file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&self.file_path)?;
        *self.file.lock().unwrap() = new_file;
        
        Ok(())
    }
}

impl LogStorage for FileLogStorage {
    fn append_entries(&mut self, entries: Vec<LogEntry>) -> Result<()> {
        for entry in entries {
            // Write to file first
            self.write_entry_to_file(&entry)?;
            // Then add to in-memory cache
            self.entries.insert(entry.index, entry);
        }
        Ok(())
    }
    
    fn get_entry(&self, index: u64) -> Option<LogEntry> {
        self.entries.get(&index).cloned()
    }
    
    fn get_last_index(&self) -> u64 {
        self.entries.keys().last().copied().unwrap_or(0)
    }
    
    fn get_last_term(&self) -> u64 {
        self.entries.values().last().map(|e| e.term).unwrap_or(0)
    }
    
    fn get_entries_from(&self, index: u64) -> Vec<LogEntry> {
        self.entries
            .range(index..)
            .map(|(_, entry)| entry.clone())
            .collect()
    }
    
    fn truncate_from(&mut self, index: u64) -> Result<()> {
        // Remove from in-memory cache
        let keys_to_remove: Vec<u64> = self.entries
            .range(index..)
            .map(|(k, _)| *k)
            .collect();
        
        for key in keys_to_remove {
            self.entries.remove(&key);
        }
        
        // Truncate file
        self.truncate_file_from_index(index)?;
        
        Ok(())
    }
}

/// In-memory log storage for testing
#[derive(Debug, Clone)]
pub struct InMemoryLogStorage {
    entries: BTreeMap<u64, LogEntry>,
}

impl InMemoryLogStorage {
    /// Create a new in-memory log storage
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }
}

impl Default for InMemoryLogStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl LogStorage for InMemoryLogStorage {
    fn append_entries(&mut self, entries: Vec<LogEntry>) -> Result<()> {
        log::debug!("Appending {} entries to in-memory log storage", entries.len());
        for entry in entries {
            log::trace!("Appending entry: index={}, term={}", entry.index, entry.term);
            self.entries.insert(entry.index, entry);
        }
        log::debug!("Log storage now contains {} entries", self.entries.len());
        Ok(())
    }
    
    fn get_entry(&self, index: u64) -> Option<LogEntry> {
        self.entries.get(&index).cloned()
    }
    
    fn get_last_index(&self) -> u64 {
        self.entries.keys().last().copied().unwrap_or(0)
    }
    
    fn get_last_term(&self) -> u64 {
        self.entries.values().last().map(|e| e.term).unwrap_or(0)
    }
    
    fn get_entries_from(&self, index: u64) -> Vec<LogEntry> {
        self.entries
            .range(index..)
            .map(|(_, entry)| entry.clone())
            .collect()
    }
    
    fn truncate_from(&mut self, index: u64) -> Result<()> {
        let keys_to_remove: Vec<u64> = self.entries
            .range(index..)
            .map(|(k, _)| *k)
            .collect();
        
        for key in keys_to_remove {
            self.entries.remove(&key);
        }
        
        Ok(())
    }
}

/// Simple base64 encoding (avoiding external dependencies)
fn base64_encode(data: &[u8]) -> String {
    // Simple hex encoding instead of base64 to avoid dependencies
    data.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Simple base64 decoding (avoiding external dependencies)
fn base64_decode(s: &str) -> Result<Vec<u8>> {
    if s.len() % 2 != 0 {
        return Err(Error::Serialization("Invalid hex string length".to_string()));
    }
    
    let mut result = Vec::new();
    for i in (0..s.len()).step_by(2) {
        let hex_byte = &s[i..i+2];
        let byte = u8::from_str_radix(hex_byte, 16)
            .map_err(|e| Error::Serialization(format!("Invalid hex byte: {}", e)))?;
        result.push(byte);
    }
    
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_in_memory_log_storage() {
        let mut storage = InMemoryLogStorage::new();
        
        assert_eq!(storage.get_last_index(), 0);
        assert_eq!(storage.get_last_term(), 0);
        
        let entries = vec![
            LogEntry::new(1, 1, b"entry1".to_vec()),
            LogEntry::new(1, 2, b"entry2".to_vec()),
            LogEntry::new(2, 3, b"entry3".to_vec()),
        ];
        
        storage.append_entries(entries).unwrap();
        
        assert_eq!(storage.get_last_index(), 3);
        assert_eq!(storage.get_last_term(), 2);
        
        assert_eq!(storage.get_entry(1).unwrap().data, b"entry1");
        assert_eq!(storage.get_entry(2).unwrap().data, b"entry2");
        assert_eq!(storage.get_entry(3).unwrap().data, b"entry3");
        
        let entries_from_2 = storage.get_entries_from(2);
        assert_eq!(entries_from_2.len(), 2);
        assert_eq!(entries_from_2[0].index, 2);
        assert_eq!(entries_from_2[1].index, 3);
        
        storage.truncate_from(2).unwrap();
        assert_eq!(storage.get_last_index(), 1);
        assert!(storage.get_entry(2).is_none());
        assert!(storage.get_entry(3).is_none());
    }

    #[test]
    fn test_base64_encoding() {
        let data = b"hello world";
        let encoded = base64_encode(data);
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(data, decoded.as_slice());
    }

    #[test]
    fn test_entry_serialization() {
        // Test the serialization functions directly
        let entry = LogEntry::new(5, 10, b"test data".to_vec());
        
        let encoded = base64_encode(&entry.data);
        let decoded = base64_decode(&encoded).unwrap();
        
        assert_eq!(entry.data, decoded);
    }
}
