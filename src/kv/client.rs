//! Key-value client implementation
//! 
//! This module provides a client interface for interacting with the
//! distributed key-value store.

use crate::{Result, Error, NodeId};
use super::{KVOperation, KVResponse};

/// Client for interacting with the key-value store
pub struct KVClient {
    /// ID of the node this client is connected to
    node_id: NodeId,
    /// Client configuration
    config: ClientConfig,
}

/// Configuration for the KV client
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Request timeout in milliseconds
    pub request_timeout_ms: u64,
    /// Maximum retry attempts
    pub max_retries: u32,
    /// Retry delay in milliseconds
    pub retry_delay_ms: u64,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            request_timeout_ms: 5000,
            max_retries: 3,
            retry_delay_ms: 100,
        }
    }
}

impl KVClient {
    /// Create a new KV client
    pub fn new(node_id: NodeId) -> Self {
        Self {
            node_id,
            config: ClientConfig::default(),
        }
    }
    
    /// Create a new KV client with custom configuration
    pub fn with_config(node_id: NodeId, config: ClientConfig) -> Self {
        Self {
            node_id,
            config,
        }
    }
    
    /// Get a value by key
    pub fn get(&self, key: String) -> Result<Option<Vec<u8>>> {
        let operation = KVOperation::Get { key };
        let response = self.send_request(operation)?;
        
        match response {
            KVResponse::Get { value, .. } => Ok(value),
            KVResponse::Error { message } => Err(Error::KeyValue(message)),
            _ => Err(Error::KeyValue("Unexpected response type".to_string())),
        }
    }
    
    /// Put a key-value pair
    pub fn put(&self, key: String, value: Vec<u8>) -> Result<()> {
        let operation = KVOperation::Put { key, value };
        let response = self.send_request(operation)?;
        
        match response {
            KVResponse::Put { .. } => Ok(()),
            KVResponse::Error { message } => Err(Error::KeyValue(message)),
            _ => Err(Error::KeyValue("Unexpected response type".to_string())),
        }
    }
    
    /// Delete a key
    pub fn delete(&self, key: String) -> Result<bool> {
        let operation = KVOperation::Delete { key };
        let response = self.send_request(operation)?;
        
        match response {
            KVResponse::Delete { .. } => Ok(false), // Stub implementation: simulate key not found
            KVResponse::Error { message } => Err(Error::KeyValue(message)),
            _ => Err(Error::KeyValue("Unexpected response type".to_string())),
        }
    }
    
    /// Get the node ID this client is connected to
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }
    
    /// List all keys
    pub fn list_keys(&self) -> Result<Vec<String>> {
        // Stub implementation - in real implementation would send request to Raft node
        // For now, return empty list
        Ok(Vec::new())
    }
    
    /// Get the client configuration
    pub fn config(&self) -> &ClientConfig {
        &self.config
    }
    
    /// Update the client configuration
    pub fn set_config(&mut self, config: ClientConfig) {
        self.config = config;
    }
    
    /// Send a request to the server and get response
    fn send_request(&self, operation: KVOperation) -> Result<KVResponse> {
        // For now, implement a simple stub that simulates server communication
        // In a full implementation, this would:
        // 1. Serialize the operation
        // 2. Send it over TCP to the server
        // 3. Wait for response
        // 4. Deserialize and return the response
        
        log::debug!("Sending KV request: {:?}", operation);
        
        // Simulate different responses based on operation type
        match operation {
            KVOperation::Get { key } => {
                log::debug!("Processing GET request for key: {}", key);
                Ok(KVResponse::Get { key, value: None })
            }
            KVOperation::Put { key, value: _ } => {
                log::debug!("Processing PUT request for key: {}", key);
                Ok(KVResponse::Put { key })
            }
            KVOperation::Delete { key } => {
                log::debug!("Processing DELETE request for key: {}", key);
                Ok(KVResponse::Delete { key })
            }
        }
    }
}

/// Mock KV client for testing
#[derive(Debug)]
pub struct MockKVClient {
    /// Responses to return for get operations
    get_responses: std::collections::HashMap<String, Option<Vec<u8>>>,
    /// Whether put operations should succeed
    put_success: bool,
    /// Whether delete operations should succeed
    delete_success: bool,
    /// Operations that have been performed
    operations: Vec<KVOperation>,
}

impl MockKVClient {
    /// Create a new mock client
    pub fn new() -> Self {
        Self {
            get_responses: std::collections::HashMap::new(),
            put_success: true,
            delete_success: true,
            operations: Vec::new(),
        }
    }
    
    /// Set the response for a get operation
    pub fn set_get_response(&mut self, key: String, value: Option<Vec<u8>>) {
        self.get_responses.insert(key, value);
    }
    
    /// Set whether put operations should succeed
    pub fn set_put_success(&mut self, success: bool) {
        self.put_success = success;
    }
    
    /// Set whether delete operations should succeed
    pub fn set_delete_success(&mut self, success: bool) {
        self.delete_success = success;
    }
    
    /// Get all operations that have been performed
    pub fn get_operations(&self) -> &[KVOperation] {
        &self.operations
    }
    
    /// Clear recorded operations
    pub fn clear_operations(&mut self) {
        self.operations.clear();
    }
    
    /// Perform a get operation
    pub fn get(&mut self, key: String) -> Result<Option<Vec<u8>>> {
        let operation = KVOperation::Get { key: key.clone() };
        self.operations.push(operation);
        
        Ok(self.get_responses.get(&key).cloned().unwrap_or(None))
    }
    
    /// Perform a put operation
    pub fn put(&mut self, key: String, value: Vec<u8>) -> Result<()> {
        let operation = KVOperation::Put { key, value };
        self.operations.push(operation);
        
        if self.put_success {
            Ok(())
        } else {
            Err(Error::KeyValue("Put operation failed".to_string()))
        }
    }
    
    /// Perform a delete operation
    pub fn delete(&mut self, key: String) -> Result<bool> {
        let operation = KVOperation::Delete { key };
        self.operations.push(operation);
        
        if self.delete_success {
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl Default for MockKVClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kv_client_creation() {
        let client = KVClient::new(1);
        assert_eq!(client.node_id(), 1);
        
        let config = client.config();
        assert_eq!(config.request_timeout_ms, 5000);
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.retry_delay_ms, 100);
    }

    #[test]
    fn test_kv_client_with_config() {
        let config = ClientConfig {
            request_timeout_ms: 10000,
            max_retries: 5,
            retry_delay_ms: 200,
        };
        
        let client = KVClient::with_config(2, config.clone());
        assert_eq!(client.node_id(), 2);
        assert_eq!(client.config().request_timeout_ms, 10000);
        assert_eq!(client.config().max_retries, 5);
        assert_eq!(client.config().retry_delay_ms, 200);
    }

    #[test]
    fn test_kv_client_operations() {
        let client = KVClient::new(1);
        
        // Test get operation (stub returns None)
        let result = client.get("test_key".to_string()).unwrap();
        assert_eq!(result, None);
        
        // Test put operation (stub returns success)
        let result = client.put("test_key".to_string(), b"test_value".to_vec());
        assert!(result.is_ok());
        
        // Test delete operation (stub returns false)
        let result = client.delete("test_key".to_string()).unwrap();
        assert_eq!(result, false);
    }

    #[test]
    fn test_mock_kv_client() {
        let mut mock_client = MockKVClient::new();
        
        // Set up mock responses
        mock_client.set_get_response("key1".to_string(), Some(b"value1".to_vec()));
        mock_client.set_get_response("key2".to_string(), None);
        
        // Test get operations
        let result1 = mock_client.get("key1".to_string()).unwrap();
        assert_eq!(result1, Some(b"value1".to_vec()));
        
        let result2 = mock_client.get("key2".to_string()).unwrap();
        assert_eq!(result2, None);
        
        let result3 = mock_client.get("key3".to_string()).unwrap();
        assert_eq!(result3, None);
        
        // Test put operation
        let put_result = mock_client.put("new_key".to_string(), b"new_value".to_vec());
        assert!(put_result.is_ok());
        
        // Test delete operation
        let delete_result = mock_client.delete("old_key".to_string()).unwrap();
        assert_eq!(delete_result, true);
        
        // Verify operations were recorded
        let operations = mock_client.get_operations();
        assert_eq!(operations.len(), 5);
        
        match &operations[0] {
            KVOperation::Get { key } => assert_eq!(key, "key1"),
            _ => panic!("Expected Get operation"),
        }
        
        match &operations[3] {
            KVOperation::Put { key, value } => {
                assert_eq!(key, "new_key");
                assert_eq!(value, b"new_value");
            }
            _ => panic!("Expected Put operation"),
        }
        
        match &operations[4] {
            KVOperation::Delete { key } => assert_eq!(key, "old_key"),
            _ => panic!("Expected Delete operation"),
        }
    }

    #[test]
    fn test_mock_client_error_conditions() {
        let mut mock_client = MockKVClient::new();
        
        // Test put failure
        mock_client.set_put_success(false);
        let put_result = mock_client.put("key".to_string(), b"value".to_vec());
        assert!(put_result.is_err());
        
        // Test delete failure
        mock_client.set_delete_success(false);
        let delete_result = mock_client.delete("key".to_string()).unwrap();
        assert_eq!(delete_result, false);
    }
}
