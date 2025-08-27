//! Key-value store implementation
//! 
//! This module provides the key-value store operations and client interface
//! that sits on top of the Raft consensus layer.

pub mod store;
pub mod client;

pub use store::InMemoryKVStore;
pub use client::KVClient;

use crate::{Result, storage::LogEntry};
use std::collections::HashMap;

/// Key-value operation types
#[derive(Debug, Clone, PartialEq)]
pub enum KVOperation {
    /// Get a value by key
    Get { key: String },
    /// Put a key-value pair
    Put { key: String, value: Vec<u8> },
    /// Delete a key
    Delete { key: String },
    /// List all keys
    List,
}

impl KVOperation {
    /// Serialize the operation to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            KVOperation::Get { key } => {
                let mut bytes = vec![0u8]; // Operation type: Get
                bytes.extend_from_slice(key.as_bytes());
                bytes
            }
            KVOperation::Put { key, value } => {
                let mut bytes = vec![1u8]; // Operation type: Put
                let key_len = key.len() as u32;
                bytes.extend_from_slice(&key_len.to_be_bytes());
                bytes.extend_from_slice(key.as_bytes());
                bytes.extend_from_slice(value);
                bytes
            }
            KVOperation::Delete { key } => {
                let mut bytes = vec![2u8]; // Operation type: Delete
                bytes.extend_from_slice(key.as_bytes());
                bytes
            }
            KVOperation::List => {
                vec![3u8] // Operation type: List
            }
        }
    }
    
    /// Deserialize operation from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Err(crate::Error::Serialization("Empty bytes".to_string()));
        }
        
        match bytes[0] {
            0 => {
                // Get operation
                let key = String::from_utf8(bytes[1..].to_vec())
                    .map_err(|e| crate::Error::Serialization(e.to_string()))?;
                Ok(KVOperation::Get { key })
            }
            1 => {
                // Put operation
                if bytes.len() < 5 {
                    return Err(crate::Error::Serialization("Invalid Put operation".to_string()));
                }
                let key_len = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
                if bytes.len() < 5 + key_len {
                    return Err(crate::Error::Serialization("Invalid Put operation length".to_string()));
                }
                let key = String::from_utf8(bytes[5..5 + key_len].to_vec())
                    .map_err(|e| crate::Error::Serialization(e.to_string()))?;
                let value = bytes[5 + key_len..].to_vec();
                Ok(KVOperation::Put { key, value })
            }
            2 => {
                // Delete operation
                let key = String::from_utf8(bytes[1..].to_vec())
                    .map_err(|e| crate::Error::Serialization(e.to_string()))?;
                Ok(KVOperation::Delete { key })
            }
            3 => {
                // List operation
                Ok(KVOperation::List)
            }
            _ => Err(crate::Error::Serialization("Unknown operation type".to_string())),
        }
    }
}

/// Response from key-value operations
#[derive(Debug, Clone, PartialEq)]
pub enum KVResponse {
    /// Successful get response
    Get { key: String, value: Option<Vec<u8>> },
    /// Successful put response
    Put { key: String },
    /// Successful delete response
    Delete { key: String },
    /// Successful list response
    List { keys: Vec<String> },
    /// Error response
    Error { message: String },
}

/// Trait for key-value storage
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
}

/// Trait for the key-value store that integrates with Raft
pub trait KVStore: Send + Sync {
    /// Apply a log entry to the state machine
    fn apply_entry(&mut self, entry: &LogEntry) -> Result<KVResponse>;
    
    /// Get current state as a snapshot
    fn snapshot(&self) -> Vec<u8>;
    
    /// Restore state from a snapshot
    fn restore_snapshot(&mut self, snapshot: &[u8]) -> Result<()>;
    
    /// Get a value directly (for read operations)
    fn get(&self, key: &str) -> Option<Vec<u8>>;
}

/// Simple in-memory key-value storage
#[derive(Debug, Clone)]
pub struct InMemoryKVStorage {
    data: HashMap<String, Vec<u8>>,
}

impl InMemoryKVStorage {
    /// Create a new in-memory storage
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kv_operation_serialization() {
        let get_op = KVOperation::Get { key: "test".to_string() };
        let bytes = get_op.to_bytes();
        let deserialized = KVOperation::from_bytes(&bytes).unwrap();
        assert_eq!(get_op, deserialized);
        
        let put_op = KVOperation::Put { 
            key: "key".to_string(), 
            value: b"value".to_vec() 
        };
        let bytes = put_op.to_bytes();
        let deserialized = KVOperation::from_bytes(&bytes).unwrap();
        assert_eq!(put_op, deserialized);
        
        let delete_op = KVOperation::Delete { key: "test".to_string() };
        let bytes = delete_op.to_bytes();
        let deserialized = KVOperation::from_bytes(&bytes).unwrap();
        assert_eq!(delete_op, deserialized);
    }

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
        
        assert!(storage.delete("key1"));
        assert!(storage.is_empty());
        assert!(!storage.contains_key("key1"));
        assert_eq!(storage.get("key1"), None);
    }
}
