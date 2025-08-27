//! Key-value store implementation that integrates with Raft
//! 
//! This module provides the main KV store that applies Raft log entries
//! to maintain a consistent key-value state machine.

use crate::{Result, Error, storage::LogEntry};
use super::{KVOperation, KVResponse, KVStore};
use crate::storage::kv_storage::{KVStorage, InMemoryKVStorage};
use std::sync::{Arc, Mutex};

/// Raft-integrated key-value store implementation
pub struct InMemoryKVStore {
    /// Underlying storage
    storage: Arc<Mutex<Box<dyn KVStorage>>>,
}

impl InMemoryKVStore {
    /// Create a new in-memory KV store
    pub fn new() -> Self {
        Self {
            storage: Arc::new(Mutex::new(Box::new(InMemoryKVStorage::new()))),
        }
    }
    
    /// Create with custom storage backend
    pub fn with_storage(storage: Box<dyn KVStorage>) -> Self {
        Self {
            storage: Arc::new(Mutex::new(storage)),
        }
    }
}

impl Default for InMemoryKVStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KVStore for InMemoryKVStore {
    fn apply_entry(&mut self, entry: &LogEntry) -> Result<KVResponse> {
        // Deserialize the operation from the log entry data
        let operation = KVOperation::from_bytes(&entry.data)?;
        
        log::debug!("Applying KV operation from log entry {}: {:?}", entry.index, operation);
        
        let mut storage = self.storage.lock().unwrap();
        
        match operation {
            KVOperation::Get { ref key } => {
                let value = storage.get(key);
                log::trace!("GET operation for key '{}': {:?}", key, value.is_some());
                Ok(KVResponse::Get { key: key.clone(), value })
            }
            KVOperation::Put { ref key, ref value } => {
                storage.put(key.clone(), value.clone());
                log::info!("PUT operation: key='{}', value_len={}", key, value.len());
                Ok(KVResponse::Put { key: key.clone() })
            }
            KVOperation::Delete { ref key } => {
                let existed = storage.contains_key(key);
                storage.delete(key);
                log::info!("DELETE operation: key='{}', existed={}", key, existed);
                Ok(KVResponse::Delete { key: key.clone() })
            }
            KVOperation::List => {
                let keys = storage.keys();
                log::trace!("LIST operation: found {} keys", keys.len());
                Ok(KVResponse::List { keys })
            }
        }
    }
    
    fn snapshot(&self) -> Vec<u8> {
        let storage = self.storage.lock().unwrap();
        
        // Simple snapshot format: serialize all key-value pairs
        let mut snapshot = Vec::new();
        
        for key in storage.keys() {
            if let Some(value) = storage.get(&key) {
                let entry_bytes = serialize_kv_entry(&key, &value);
                snapshot.extend_from_slice(&(entry_bytes.len() as u32).to_be_bytes());
                snapshot.extend_from_slice(&entry_bytes);
            }
        }
        
        snapshot
    }
    
    fn restore_snapshot(&mut self, snapshot: &[u8]) -> Result<()> {
        let mut storage = self.storage.lock().unwrap();
        storage.clear();
        
        let mut offset = 0;
        while offset < snapshot.len() {
            if offset + 4 > snapshot.len() {
                return Err(Error::Serialization("Invalid snapshot format".to_string()));
            }
            
            let entry_len = u32::from_be_bytes([
                snapshot[offset],
                snapshot[offset + 1],
                snapshot[offset + 2],
                snapshot[offset + 3],
            ]) as usize;
            offset += 4;
            
            if offset + entry_len > snapshot.len() {
                return Err(Error::Serialization("Invalid snapshot entry length".to_string()));
            }
            
            let entry_bytes = &snapshot[offset..offset + entry_len];
            let (key, value) = deserialize_kv_entry(entry_bytes)?;
            storage.put(key, value);
            
            offset += entry_len;
        }
        
        Ok(())
    }
    
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        let storage = self.storage.lock().unwrap();
        storage.get(key)
    }
}

impl InMemoryKVStore {
    /// Put a key-value pair directly (for server-side operations)
    pub fn put(&mut self, key: String, value: Vec<u8>) {
        let mut storage = self.storage.lock().unwrap();
        storage.put(key, value);
    }
    
    /// Delete a key directly (for server-side operations)
    pub fn delete(&mut self, key: &str) -> bool {
        let mut storage = self.storage.lock().unwrap();
        storage.delete(key)
    }
    
    /// List all keys in the store
    pub fn list_keys(&self) -> Vec<String> {
        let storage = self.storage.lock().unwrap();
        storage.keys()
    }
}

/// Serialize a key-value pair for snapshots
fn serialize_kv_entry(key: &str, value: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    
    // Key length and key
    bytes.extend_from_slice(&(key.len() as u32).to_be_bytes());
    bytes.extend_from_slice(key.as_bytes());
    
    // Value length and value
    bytes.extend_from_slice(&(value.len() as u32).to_be_bytes());
    bytes.extend_from_slice(value);
    
    bytes
}

/// Deserialize a key-value pair from snapshot data
fn deserialize_kv_entry(bytes: &[u8]) -> Result<(String, Vec<u8>)> {
    if bytes.len() < 8 {
        return Err(Error::Serialization("Invalid KV entry format".to_string()));
    }
    
    let mut offset = 0;
    
    // Read key length
    let key_len = u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]) as usize;
    offset += 4;
    
    if offset + key_len > bytes.len() {
        return Err(Error::Serialization("Invalid key length".to_string()));
    }
    
    // Read key
    let key = String::from_utf8(bytes[offset..offset + key_len].to_vec())
        .map_err(|e| Error::Serialization(format!("Invalid key UTF-8: {}", e)))?;
    offset += key_len;
    
    if offset + 4 > bytes.len() {
        return Err(Error::Serialization("Invalid value length position".to_string()));
    }
    
    // Read value length
    let value_len = u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]) as usize;
    offset += 4;
    
    if offset + value_len > bytes.len() {
        return Err(Error::Serialization("Invalid value length".to_string()));
    }
    
    // Read value
    let value = bytes[offset..offset + value_len].to_vec();
    
    Ok((key, value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::LogEntry;

    #[test]
    fn test_kv_store_creation() {
        let store = InMemoryKVStore::new();
        assert!(store.get("nonexistent").is_none());
    }

    #[test]
    fn test_kv_store_operations() {
        let mut store = InMemoryKVStore::new();
        
        // Test PUT operation
        let put_op = KVOperation::Put {
            key: "test_key".to_string(),
            value: b"test_value".to_vec(),
        };
        let put_entry = LogEntry::new(1, 1, put_op.to_bytes());
        let put_response = store.apply_entry(&put_entry).unwrap();
        
        match put_response {
            KVResponse::Put { key } => assert_eq!(key, "test_key"),
            _ => panic!("Expected Put response"),
        }
        
        // Test GET operation
        let get_op = KVOperation::Get {
            key: "test_key".to_string(),
        };
        let get_entry = LogEntry::new(1, 2, get_op.to_bytes());
        let get_response = store.apply_entry(&get_entry).unwrap();
        
        match get_response {
            KVResponse::Get { key, value } => {
                assert_eq!(key, "test_key");
                assert_eq!(value, Some(b"test_value".to_vec()));
            }
            _ => panic!("Expected Get response"),
        }
        
        // Test DELETE operation
        let delete_op = KVOperation::Delete {
            key: "test_key".to_string(),
        };
        let delete_entry = LogEntry::new(1, 3, delete_op.to_bytes());
        let delete_response = store.apply_entry(&delete_entry).unwrap();
        
        match delete_response {
            KVResponse::Delete { key } => assert_eq!(key, "test_key"),
            _ => panic!("Expected Delete response"),
        }
        
        // Verify key is deleted
        assert!(store.get("test_key").is_none());
    }

    #[test]
    fn test_snapshot_and_restore() {
        let mut store = InMemoryKVStore::new();
        
        // Add some data
        let put_op1 = KVOperation::Put {
            key: "key1".to_string(),
            value: b"value1".to_vec(),
        };
        let put_op2 = KVOperation::Put {
            key: "key2".to_string(),
            value: b"value2".to_vec(),
        };
        
        store.apply_entry(&LogEntry::new(1, 1, put_op1.to_bytes())).unwrap();
        store.apply_entry(&LogEntry::new(1, 2, put_op2.to_bytes())).unwrap();
        
        // Create snapshot
        let snapshot = store.snapshot();
        assert!(!snapshot.is_empty());
        
        // Create new store and restore from snapshot
        let mut new_store = InMemoryKVStore::new();
        new_store.restore_snapshot(&snapshot).unwrap();
        
        // Verify data is restored
        assert_eq!(new_store.get("key1"), Some(b"value1".to_vec()));
        assert_eq!(new_store.get("key2"), Some(b"value2".to_vec()));
    }

    #[test]
    fn test_kv_entry_serialization() {
        let key = "test_key";
        let value = b"test_value";
        
        let serialized = serialize_kv_entry(key, value);
        let (deserialized_key, deserialized_value) = deserialize_kv_entry(&serialized).unwrap();
        
        assert_eq!(deserialized_key, key);
        assert_eq!(deserialized_value, value);
    }
}
