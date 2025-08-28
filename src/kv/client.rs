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
    /// Cluster addresses for server connections
    cluster_addresses: Vec<String>,
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
    /// Create a new KV client with cluster addresses
    /// This is the primary constructor - cluster addresses must be provided
    pub fn with_cluster_addresses(node_id: NodeId, cluster_addresses: Vec<String>) -> Self {
        // Use cluster addresses exactly as provided from command line
        Self {
            node_id,
            config: ClientConfig::default(),
            cluster_addresses,
        }
    }
    
    /// Create a new KV client with custom configuration and cluster addresses
    pub fn with_config_and_addresses(node_id: NodeId, config: ClientConfig, cluster_addresses: Vec<String>) -> Self {
        // Use cluster addresses exactly as provided from command line
        Self {
            node_id,
            config,
            cluster_addresses,
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
        let operation = KVOperation::List;
        let response = self.send_request(operation)?;
        
        match response {
            KVResponse::List { keys } => Ok(keys),
            KVResponse::Error { message } => Err(Error::KeyValue(message)),
            _ => Err(Error::KeyValue("Unexpected response type".to_string())),
        }
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
        use std::net::TcpStream;
        use std::io::{Read, Write, BufReader, BufWriter};
        use std::time::Duration;
        
        log::debug!("Sending KV request: {:?}", operation);
        
        // Check if we have any cluster addresses
        if self.cluster_addresses.is_empty() {
            return Err(Error::Network("No cluster addresses configured".to_string()));
        }
        
        // Try each server in the cluster until we find one that responds
        // In a Raft cluster, only the leader can process client requests
        let mut last_error = None;
        
        for (index, server_address) in self.cluster_addresses.iter().enumerate() {
            log::info!("Client connecting to server at {} (this should be the leader)", server_address);
            
            match self.try_server_request(&operation, server_address) {
                Ok(response) => {
                    log::info!("Successfully connected to leader at {}", server_address);
                    return Ok(response);
                }
                Err(e) => {
                    log::warn!("Failed to connect to server at {}: {}", server_address, e);
                    last_error = Some(e);
                    
                    // If this isn't the last server, try the next one
                    if index < self.cluster_addresses.len() - 1 {
                        log::debug!("Trying next server in cluster...");
                        continue;
                    }
                }
            }
        }
        
        // If we get here, all servers failed
        Err(last_error.unwrap_or_else(|| Error::Network("No servers available".to_string())))
    }
    
    /// Try to send a request to a specific server
    fn try_server_request(&self, operation: &KVOperation, server_address: &str) -> Result<KVResponse> {
        use std::net::TcpStream;
        use std::io::{Read, Write, BufReader, BufWriter};
        use std::time::Duration;
        
        // Connect to server with timeout
        let stream = TcpStream::connect_timeout(
            &server_address.parse().map_err(|e| Error::Network(format!("Invalid server address: {}", e)))?,
            Duration::from_secs(5)
        ).map_err(|e| Error::Network(format!("Failed to connect to server: {}", e)))?;
        
        let mut writer = BufWriter::new(&stream);
        let mut reader = BufReader::new(&stream);
        
        // Serialize the operation
        let operation_bytes = operation.to_bytes();
        
        // Send framed message: 4 bytes length + operation data
        let message_len = operation_bytes.len() as u32;
        let mut framed_message = Vec::with_capacity(4 + operation_bytes.len());
        framed_message.extend_from_slice(&message_len.to_be_bytes());
        framed_message.extend_from_slice(&operation_bytes);
        
        // Send the request
        writer.write_all(&framed_message)
            .map_err(|e| Error::Network(format!("Failed to send request: {}", e)))?;
        writer.flush()
            .map_err(|e| Error::Network(format!("Failed to flush request: {}", e)))?;
        
        log::debug!("Sent {} byte request to server", operation_bytes.len());
        
        // Read response header (4 bytes for length)
        let mut header = [0u8; 4];
        reader.read_exact(&mut header)
            .map_err(|e| Error::Network(format!("Failed to read response header: {}", e)))?;
        
        let response_len = u32::from_be_bytes(header) as usize;
        
        // Validate response length
        if response_len > 1024 * 1024 { // 1MB limit
            return Err(Error::Network("Response too large".to_string()));
        }
        
        // Read response data
        let mut response_data = vec![0u8; response_len];
        reader.read_exact(&mut response_data)
            .map_err(|e| Error::Network(format!("Failed to read response data: {}", e)))?;
        
        log::debug!("Received {} byte response from server", response_len);
        
        // Deserialize the response
        self.deserialize_response(&response_data)
    }
    
    /// Deserialize a response from bytes
    fn deserialize_response(&self, bytes: &[u8]) -> Result<KVResponse> {
        if bytes.is_empty() {
            return Err(Error::Serialization("Empty response".to_string()));
        }
        
        match bytes[0] {
            0 => {
                // Get response
                if bytes.len() < 5 {
                    return Err(Error::Serialization("Invalid Get response".to_string()));
                }
                let key_len = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
                if bytes.len() < 5 + key_len + 1 {
                    return Err(Error::Serialization("Invalid Get response length".to_string()));
                }
                let key = String::from_utf8(bytes[5..5 + key_len].to_vec())
                    .map_err(|e| Error::Serialization(e.to_string()))?;
                let has_value = bytes[5 + key_len] != 0;
                let value = if has_value {
                    Some(bytes[5 + key_len + 1..].to_vec())
                } else {
                    None
                };
                Ok(KVResponse::Get { key, value })
            }
            1 => {
                // Put response
                if bytes.len() < 5 {
                    return Err(Error::Serialization("Invalid Put response".to_string()));
                }
                let key_len = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
                if bytes.len() < 5 + key_len {
                    return Err(Error::Serialization("Invalid Put response length".to_string()));
                }
                let key = String::from_utf8(bytes[5..5 + key_len].to_vec())
                    .map_err(|e| Error::Serialization(e.to_string()))?;
                Ok(KVResponse::Put { key })
            }
            2 => {
                // Delete response
                if bytes.len() < 5 {
                    return Err(Error::Serialization("Invalid Delete response".to_string()));
                }
                let key_len = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
                if bytes.len() < 5 + key_len {
                    return Err(Error::Serialization("Invalid Delete response length".to_string()));
                }
                let key = String::from_utf8(bytes[5..5 + key_len].to_vec())
                    .map_err(|e| Error::Serialization(e.to_string()))?;
                Ok(KVResponse::Delete { key })
            }
            3 => {
                // Error response
                let message = String::from_utf8(bytes[1..].to_vec())
                    .map_err(|e| Error::Serialization(e.to_string()))?;
                Ok(KVResponse::Error { message })
            }
            4 => {
                // List response
                if bytes.len() < 5 {
                    return Err(Error::Serialization("Invalid List response".to_string()));
                }
                let key_count = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
                let mut keys = Vec::with_capacity(key_count);
                let mut offset = 5;
                
                for _ in 0..key_count {
                    if offset + 4 > bytes.len() {
                        return Err(Error::Serialization("Invalid List response key length".to_string()));
                    }
                    let key_len = u32::from_be_bytes([
                        bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]
                    ]) as usize;
                    offset += 4;
                    
                    if offset + key_len > bytes.len() {
                        return Err(Error::Serialization("Invalid List response key data".to_string()));
                    }
                    let key = String::from_utf8(bytes[offset..offset + key_len].to_vec())
                        .map_err(|e| Error::Serialization(e.to_string()))?;
                    keys.push(key);
                    offset += key_len;
                }
                
                Ok(KVResponse::List { keys })
            }
            _ => Err(Error::Serialization("Unknown response type".to_string())),
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
        let cluster_addresses = vec!["127.0.0.1:8080".to_string()];
        let client = KVClient::with_cluster_addresses(1, cluster_addresses);
        assert_eq!(client.node_id(), 1);
        
        let config = client.config();
        assert_eq!(config.request_timeout_ms, 5000);
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.retry_delay_ms, 100);
        
        // Verify cluster addresses are used exactly as provided
        assert_eq!(client.cluster_addresses, vec!["127.0.0.1:8080".to_string()]);
    }

    #[test]
    fn test_kv_client_with_config() {
        let config = ClientConfig {
            request_timeout_ms: 10000,
            max_retries: 5,
            retry_delay_ms: 200,
        };
        
        let cluster_addresses = vec!["127.0.0.1:8080".to_string()];
        let client = KVClient::with_config_and_addresses(2, config.clone(), cluster_addresses);
        assert_eq!(client.node_id(), 2);
        assert_eq!(client.config().request_timeout_ms, 10000);
        assert_eq!(client.config().max_retries, 5);
        assert_eq!(client.config().retry_delay_ms, 200);
        
        // Verify cluster addresses are used exactly as provided
        assert_eq!(client.cluster_addresses, vec!["127.0.0.1:8080".to_string()]);
    }

    #[test]
    fn test_kv_client_operations_no_addresses() {
        let client = KVClient::with_cluster_addresses(1, vec![]);
        
        // Test operations should fail with no cluster addresses
        let result = client.get("test_key".to_string());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No cluster addresses configured"));
        
        let result = client.put("test_key".to_string(), b"test_value".to_vec());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No cluster addresses configured"));
        
        let result = client.delete("test_key".to_string());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No cluster addresses configured"));
    }

    #[test]
    fn test_kv_client_exact_addresses() {
        // Test that various address formats are used exactly as provided
        let cluster_addresses = vec![
            "127.0.0.1:8080".to_string(),
            "localhost:8081".to_string(),
            "192.168.1.1:8082".to_string(),
        ];
        let client = KVClient::with_cluster_addresses(1, cluster_addresses.clone());
        
        // Addresses should be used exactly as provided (no port conversion)
        assert_eq!(client.cluster_addresses, cluster_addresses);
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
