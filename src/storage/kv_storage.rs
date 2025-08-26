//! Key-value storage implementations
//! 
//! This module provides storage implementations for the key-value data
//! that can be used by the Raft state machine.

use crate::{Result, Error};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Trait for key-value storage backend
pub trait KVStorage: Send + Sync {
    /// Get a value by key
    fn get(&self, key: &str) -> Option<Vec<u8>>;
    
    /// Put a key-value pair
    fn put(&mut self, key: String, value: Vec<u8>);
    
    /// Delete a key
    fn delete(&mut self, key: &str) -> bool;
    
    /// Get all keys
    fn keys(&self) -> Vec<String>;
    
    /// Check if key exists
    fn contains_key(&self, key: &str) -> bool;
    
    /// Get number of key-value pairs
    fn len(&self) -> usize;
    
    /// Check if storage is empty
    fn is_empty(&self) -> bool;
    
    /// Clear all data
    fn clear(&mut self);
}

/// File-based key-value storage implementation
pub struct FileKVStorage {
    /// Path to the storage file
    file_path: PathBuf,
    /// In-memory cache for fast access
    data: HashMap<String, Vec<u8>>,
    /// File handle for writing
    file: Arc<Mutex<File>>,
}

impl FileKVStorage {
    /// Create a new file-based KV storage
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
            data: HashMap::new(),
            file: Arc::new(Mutex::new(file)),
        };
        
        // Load existing data from file
        storage.load_from_file()?;
        
        Ok(storage)
    }
    
    /// Load data from the storage file
    fn load_from_file(&mut self) -> Result<()> {
        let file = File::open(&self.file_path)?;
        let reader = BufReader::new(file);
        
        for line in reader.lines() {
            let line = line?;
            if let Ok((key, value)) = self.deserialize_entry(&line) {
                if value.is_empty() {
                    // Empty value means deletion
                    self.data.remove(&key);
                } else {
                    self.data.insert(key, value);
                }
            }
        }
        
        Ok(())
    }
    
    /// Serialize a key-value entry to a string
    fn serialize_entry(&self, key: &str, value: &[u8]) -> String {
        // Simple format: key_len:key:value_len:value_hex
        let value_hex = hex_encode(value);
        format!("{}:{}:{}:{}\n", key.len(), key, value.len(), value_hex)
    }
    
    /// Serialize a deletion entry
    fn serialize_deletion(&self, key: &str) -> String {
        // Deletion is represented as empty value
        format!("{}:{}:0:\n", key.len(), key)
    }
    
    /// Deserialize a key-value entry from a string
    fn deserialize_entry(&self, line: &str) -> Result<(String, Vec<u8>)> {
        let parts: Vec<&str> = line.trim().split(':').collect();
        if parts.len() != 4 {
            return Err(Error::Serialization("Invalid KV entry format".to_string()));
        }
        
        let key_len = parts[0].parse::<usize>()
            .map_err(|e| Error::Serialization(format!("Invalid key length: {}", e)))?;
        
        if parts[1].len() != key_len {
            return Err(Error::Serialization("Key length mismatch".to_string()));
        }
        let key = parts[1].to_string();
        
        let value_len = parts[2].parse::<usize>()
            .map_err(|e| Error::Serialization(format!("Invalid value length: {}", e)))?;
        
        let value = if value_len == 0 {
            Vec::new() // Deletion marker
        } else {
            hex_decode(parts[3])
                .map_err(|e| Error::Serialization(format!("Invalid hex value: {}", e)))?
        };
        
        if value.len() != value_len {
            return Err(Error::Serialization("Value length mismatch".to_string()));
        }
        
        Ok((key, value))
    }
    
    /// Write entry to file
    fn write_entry_to_file(&self, key: &str, value: &[u8]) -> Result<()> {
        let serialized = self.serialize_entry(key, value);
        let mut file = self.file.lock().unwrap();
        file.write_all(serialized.as_bytes())?;
        file.flush()?;
        Ok(())
    }
    
    /// Write deletion to file
    fn write_deletion_to_file(&self, key: &str) -> Result<()> {
        let serialized = self.serialize_deletion(key);
        let mut file = self.file.lock().unwrap();
        file.write_all(serialized.as_bytes())?;
        file.flush()?;
        Ok(())
    }
}

impl KVStorage for FileKVStorage {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.data.get(key).cloned()
    }
    
    fn put(&mut self, key: String, value: Vec<u8>) {
        // Write to file first
        if let Err(_) = self.write_entry_to_file(&key, &value) {
            // Handle error - for now just continue
            // In production, this should be handled properly
        }
        // Then update in-memory cache
        self.data.insert(key, value);
    }
    
    fn delete(&mut self, key: &str) -> bool {
        if self.data.contains_key(key) {
            // Write deletion to file first
            if let Err(_) = self.write_deletion_to_file(key) {
                // Handle error - for now just continue
            }
            // Then remove from in-memory cache
            self.data.remove(key).is_some()
        } else {
            false
        }
    }
    
    fn keys(&self) -> Vec<String> {
        self.data.keys().cloned().collect()
    }
    
    fn contains_key(&self, key: &str) -> bool {
        self.data.contains_key(key)
    }
    
    fn len(&self) -> usize {
        self.data.len()
    }
    
    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    
    fn clear(&mut self) {
        // For file storage, we'd need to truncate the file
        // For now, just clear the in-memory cache
        self.data.clear();
    }
}

/// In-memory key-value storage for testing
#[derive(Debug, Clone)]
pub struct InMemoryKVStorage {
    data: HashMap<String, Vec<u8>>,
}

impl InMemoryKVStorage {
    /// Create a new in-memory KV storage
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
        }
    }
}

impl Default for InMemoryKVStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl KVStorage for InMemoryKVStorage {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.data.get(key).cloned()
    }
    
    fn put(&mut self, key: String, value: Vec<u8>) {
        self.data.insert(key, value);
    }
    
    fn delete(&mut self, key: &str) -> bool {
        self.data.remove(key).is_some()
    }
    
    fn keys(&self) -> Vec<String> {
        self.data.keys().cloned().collect()
    }
    
    fn contains_key(&self, key: &str) -> bool {
        self.data.contains_key(key)
    }
    
    fn len(&self) -> usize {
        self.data.len()
    }
    
    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    
    fn clear(&mut self) {
        self.data.clear();
    }
}

/// Simple hex encoding
fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Simple hex decoding
fn hex_decode(s: &str) -> Result<Vec<u8>> {
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
    fn test_in_memory_kv_storage() {
        let mut storage = InMemoryKVStorage::new();
        
        assert!(storage.is_empty());
        assert_eq!(storage.len(), 0);
        
        storage.put("key1".to_string(), b"value1".to_vec());
        assert!(!storage.is_empty());
        assert_eq!(storage.len(), 1);
        assert!(storage.contains_key("key1"));
        assert_eq!(storage.get("key1"), Some(b"value1".to_vec()));
        
        storage.put("key2".to_string(), b"value2".to_vec());
        assert_eq!(storage.len(), 2);
        
        let keys = storage.keys();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"key1".to_string()));
        assert!(keys.contains(&"key2".to_string()));
        
        assert!(storage.delete("key1"));
        assert_eq!(storage.len(), 1);
        assert!(!storage.contains_key("key1"));
        assert_eq!(storage.get("key1"), None);
        
        assert!(!storage.delete("nonexistent"));
        
        storage.clear();
        assert!(storage.is_empty());
        assert_eq!(storage.len(), 0);
    }

    #[test]
    fn test_hex_encoding() {
        let data = b"hello world";
        let encoded = hex_encode(data);
        let decoded = hex_decode(&encoded).unwrap();
        assert_eq!(data, decoded.as_slice());
        
        // Test empty data
        let empty_encoded = hex_encode(&[]);
        let empty_decoded = hex_decode(&empty_encoded).unwrap();
        assert_eq!(empty_decoded, Vec::<u8>::new());
    }

    #[test]
    fn test_entry_serialization() {
        // Test hex encoding/decoding functions directly
        let data = b"test_value";
        let encoded = hex_encode(data);
        let decoded = hex_decode(&encoded).unwrap();
        assert_eq!(data, decoded.as_slice());
        
        // Test with special characters
        let special_data = b"test\x00\xff\x42value";
        let special_encoded = hex_encode(special_data);
        let special_decoded = hex_decode(&special_encoded).unwrap();
        assert_eq!(special_data, special_decoded.as_slice());
    }

    #[test]
    fn test_file_kv_storage() {
        let temp_dir = tempdir().unwrap();
        let file_path = temp_dir.path().join("kv.txt");
        
        // Create storage and add some data
        {
            let mut storage = FileKVStorage::new(file_path.clone()).unwrap();
            storage.put("key1".to_string(), b"value1".to_vec());
            storage.put("key2".to_string(), b"value2".to_vec());
            storage.delete("key1");
        }
        
        // Create new storage instance and verify persistence
        {
            let storage = FileKVStorage::new(file_path).unwrap();
            assert_eq!(storage.get("key1"), None); // Was deleted
            assert_eq!(storage.get("key2"), Some(b"value2".to_vec()));
            assert_eq!(storage.len(), 1);
        }
    }
}
