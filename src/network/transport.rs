//! Network transport implementations
//! 
//! This module provides network transport for inter-node communication
//! with both TCP-based and mock implementations for testing.

use crate::{Result, Error, NodeId};
use super::NetworkConfig;
use crate::kv::KVStore;

/// Trait for network transport
pub trait NetworkTransport: Send + Sync {
    /// Send a message to a specific node
    fn send_message(&self, to: NodeId, message: Vec<u8>) -> Result<()>;
    
    /// Receive pending messages
    fn receive_messages(&self) -> Vec<(NodeId, Vec<u8>)>;
    
    /// Check if connected to a node
    fn is_connected(&self, node_id: NodeId) -> bool;
    
    /// Get list of connected nodes
    fn connected_nodes(&self) -> Vec<NodeId>;
    
    /// Get as Any for downcasting
    fn as_any(&self) -> &dyn std::any::Any;
}
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::net::{TcpListener, TcpStream, SocketAddr};
use std::io::{Read, Write, BufReader, BufWriter};
use std::thread;
use std::time::Duration;

/// TCP-based network transport implementation with dual-port support
pub struct TcpTransport {
    /// Network configuration
    config: NetworkConfig,
    /// TCP listener for Raft inter-node communication
    raft_listener: Option<TcpListener>,
    /// TCP listener for client requests
    client_listener: Option<TcpListener>,
    /// Connected nodes with their TCP streams
    connections: Arc<Mutex<HashMap<NodeId, TcpStream>>>,
    /// Incoming Raft message queue
    incoming_messages: Arc<Mutex<Vec<(NodeId, Vec<u8>)>>>,
    /// Client request queue
    client_requests: Arc<Mutex<Vec<(std::net::TcpStream, Vec<u8>)>>>,
    /// Node ID of this transport
    node_id: NodeId,
    /// Whether the Raft listener thread is running
    raft_listener_running: Arc<Mutex<bool>>,
    /// Whether the client listener thread is running
    client_listener_running: Arc<Mutex<bool>>,
}

impl TcpTransport {
    /// Get pending client requests that need to be processed
    pub fn get_client_requests(&self) -> Vec<(TcpStream, Vec<u8>)> {
        let mut requests = self.client_requests.lock().unwrap();
        let mut result = Vec::new();
        // Move the requests out instead of cloning (TcpStream doesn't implement Clone)
        while let Some(request) = requests.pop() {
            result.push(request);
        }
        result
    }
    
    /// Create a new TCP transport with dual-port support
    pub fn new(config: NetworkConfig, node_id: NodeId, bind_address: &str) -> Result<Self> {
        // Parse bind address to get the Raft port
        let bind_addr: SocketAddr = bind_address.parse()
            .map_err(|e| Error::Network(format!("Invalid bind address '{}': {}", bind_address, e)))?;
        
        // Create Raft listener on the specified port
        let raft_listener = TcpListener::bind(bind_addr)
            .map_err(|e| Error::Network(format!("Failed to bind Raft listener to {}: {}", bind_addr, e)))?;
        
        // Set non-blocking mode for the Raft listener
        raft_listener.set_nonblocking(true)
            .map_err(|e| Error::Network(format!("Failed to set non-blocking mode for Raft listener: {}", e)))?;
        
        // Create client listener on port + 1000
        let client_port = bind_addr.port() + 1000;
        let client_bind_addr = SocketAddr::new(bind_addr.ip(), client_port);
        let client_listener = TcpListener::bind(client_bind_addr)
            .map_err(|e| Error::Network(format!("Failed to bind client listener to {}: {}", client_bind_addr, e)))?;
        
        // Set non-blocking mode for the client listener
        client_listener.set_nonblocking(true)
            .map_err(|e| Error::Network(format!("Failed to set non-blocking mode for client listener: {}", e)))?;
        
        log::info!("TCP transport: Raft listener on {} for node {}", bind_addr, node_id);
        log::info!("TCP transport: Client listener on {} for node {}", client_bind_addr, node_id);
        
        Ok(Self {
            config,
            raft_listener: Some(raft_listener),
            client_listener: Some(client_listener),
            connections: Arc::new(Mutex::new(HashMap::new())),
            incoming_messages: Arc::new(Mutex::new(Vec::new())),
            client_requests: Arc::new(Mutex::new(Vec::new())),
            node_id,
            raft_listener_running: Arc::new(Mutex::new(false)),
            client_listener_running: Arc::new(Mutex::new(false)),
        })
    }
    
    /// Start accepting incoming connections on both Raft and client listeners
    pub fn start_listener(&self) -> Result<()> {
        // Start Raft listener thread
        if let Some(ref raft_listener) = self.raft_listener {
            let raft_listener_clone = raft_listener.try_clone()
                .map_err(|e| Error::Network(format!("Failed to clone Raft listener: {}", e)))?;
            let incoming_messages = Arc::clone(&self.incoming_messages);
            let node_id = self.node_id;
            
            thread::spawn(move || {
                log::debug!("TCP transport: started Raft listener thread for node {}", node_id);
                
                loop {
                    match raft_listener_clone.accept() {
                        Ok((mut stream, addr)) => {
                            log::info!("TCP transport: accepted Raft connection from {}", addr);
                            
                            // Handle Raft connection in a separate thread
                            let incoming_messages_clone = Arc::clone(&incoming_messages);
                            thread::spawn(move || {
                                Self::handle_raft_connection(&mut stream, incoming_messages_clone, node_id);
                            });
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            // No pending connections, sleep briefly
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(e) => {
                            log::error!("TCP transport: error accepting Raft connection: {}", e);
                            thread::sleep(Duration::from_millis(100));
                        }
                    }
                }
            });
        }
        
        // Start client listener thread
        if let Some(ref client_listener) = self.client_listener {
            let client_listener_clone = client_listener.try_clone()
                .map_err(|e| Error::Network(format!("Failed to clone client listener: {}", e)))?;
            let client_requests = Arc::clone(&self.client_requests);
            let node_id = self.node_id;
            
            thread::spawn(move || {
                log::debug!("TCP transport: started client listener thread for node {}", node_id);
                
                loop {
                    match client_listener_clone.accept() {
                        Ok((mut stream, addr)) => {
                            log::info!("TCP transport: accepted client connection from {}", addr);
                            
                            // Handle client connection in a separate thread
                            let client_requests_clone = Arc::clone(&client_requests);
                            thread::spawn(move || {
                                Self::handle_client_connection(&mut stream, client_requests_clone, node_id);
                            });
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            // No pending connections, sleep briefly
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(e) => {
                            log::error!("TCP transport: error accepting client connection: {}", e);
                            thread::sleep(Duration::from_millis(100));
                        }
                    }
                }
            });
        }
        
        Ok(())
    }
    
    /// Handle an incoming Raft connection (dedicated to Raft inter-node communication)
    fn handle_raft_connection(
        stream: &mut TcpStream, 
        incoming_messages: Arc<Mutex<Vec<(NodeId, Vec<u8>)>>>,
        node_id: NodeId
    ) {
        log::debug!("TCP transport: handling Raft connection for node {}", node_id);
        
        // Set stream to blocking mode for easier message parsing
        stream.set_nonblocking(false).unwrap_or_else(|e| {
            log::error!("TCP transport: failed to set blocking mode: {}", e);
        });
        
        let mut reader = BufReader::new(stream);
        let mut message_buffer = Vec::new();
        
        loop {
            // First, read just the 4-byte length header
            let mut length_buffer = [0u8; 4];
            match reader.read_exact(&mut length_buffer) {
                Ok(()) => {
                    let message_len = u32::from_be_bytes(length_buffer) as usize;
                    
                    // Validate message length
                    if message_len > 1024 * 1024 { // 1MB limit
                        log::warn!("TCP transport: Raft message too large ({} bytes), closing connection", message_len);
                        break;
                    }
                    
                    // Read the complete message data
                    message_buffer.clear();
                    message_buffer.resize(message_len, 0);
                    
                    match reader.read_exact(&mut message_buffer) {
                        Ok(()) => {
                            log::debug!("TCP transport: received {} byte Raft message", message_len);
                            
                            // Raft messages have format: node_id (8 bytes) + raft_message_data
                            if message_buffer.len() >= 8 {
                                let sender_node_id = u64::from_be_bytes([
                                    message_buffer[0], message_buffer[1], message_buffer[2], message_buffer[3],
                                    message_buffer[4], message_buffer[5], message_buffer[6], message_buffer[7]
                                ]);
                                
                                let raft_message_data = message_buffer[8..].to_vec();
                                
                                log::debug!("TCP transport: processed Raft message from node {} ({} bytes)", 
                                           sender_node_id, raft_message_data.len());
                                
                                // Add to incoming message queue
                                incoming_messages.lock().unwrap().push((sender_node_id, raft_message_data));
                            } else {
                                log::warn!("TCP transport: invalid Raft message format (too short)");
                                break;
                            }
                        }
                        Err(e) => {
                            log::error!("TCP transport: error reading Raft message data: {}", e);
                            break;
                        }
                    }
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::UnexpectedEof {
                        log::info!("TCP transport: Raft connection closed by peer");
                    } else {
                        log::error!("TCP transport: error reading Raft message length: {}", e);
                    }
                    break;
                }
            }
        }
        
        log::debug!("TCP transport: Raft connection handler exiting for node {}", node_id);
    }
    
    /// Handle an incoming client connection (dedicated to client requests)
    fn handle_client_connection(
        stream: &mut TcpStream, 
        _client_requests: Arc<Mutex<Vec<(TcpStream, Vec<u8>)>>>,
        node_id: NodeId
    ) {
        log::debug!("TCP transport: handling client connection for node {}", node_id);
        
        // Set stream to blocking mode for easier message parsing
        stream.set_nonblocking(false).unwrap_or_else(|e| {
            log::error!("TCP transport: failed to set blocking mode: {}", e);
        });
        
        // Clone the stream for response writing before creating the reader
        let mut write_stream = match stream.try_clone() {
            Ok(s) => s,
            Err(e) => {
                log::error!("TCP transport: failed to clone client stream: {}", e);
                return;
            }
        };
        
        let mut reader = BufReader::new(stream);
        let mut message_buffer = Vec::new();
        
        // Client connections are typically one request-response cycles
        // First, read just the 4-byte length header
        let mut length_buffer = [0u8; 4];
        match reader.read_exact(&mut length_buffer) {
            Ok(()) => {
                let message_len = u32::from_be_bytes(length_buffer) as usize;
                
                // Validate message length
                if message_len > 1024 * 1024 { // 1MB limit
                    log::warn!("TCP transport: client message too large ({} bytes), closing connection", message_len);
                    return;
                }
                
                // Read the complete message data
                message_buffer.resize(message_len, 0);
                
                match reader.read_exact(&mut message_buffer) {
                    Ok(()) => {
                        log::debug!("TCP transport: received {} byte client request", message_len);
                        
                        // Process the request immediately and send response
                        // This is a simplified approach - we'll send a basic response for now
                        if let Err(e) = Self::send_immediate_client_response(&mut write_stream, &message_buffer, node_id) {
                            log::error!("TCP transport: failed to send immediate response: {}", e);
                        }
                        
                        log::debug!("TCP transport: processed client request immediately");
                    }
                    Err(e) => {
                        log::error!("TCP transport: error reading client message data: {}", e);
                    }
                }
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    log::info!("TCP transport: client connection closed by peer");
                } else {
                    log::error!("TCP transport: error reading client message length: {}", e);
                }
            }
        }
        
        log::debug!("TCP transport: client connection handler exiting for node {}", node_id);
    }
    
    /// Send an immediate response to a client request (simplified for now)
    fn send_immediate_client_response(
        stream: &mut TcpStream,
        request_data: &[u8],
        node_id: NodeId
    ) -> std::result::Result<(), std::io::Error> {
        use crate::kv::{KVOperation, KVResponse};
        use std::io::Write;
        
        log::debug!("TCP transport: processing immediate client response for node {}", node_id);
        
        // Try to deserialize the request
        let response = match KVOperation::from_bytes(request_data) {
            Ok(operation) => {
                log::debug!("TCP transport: processing operation: {:?}", operation);
                
                // For now, send basic responses (this is a temporary fix)
                match operation {
                    KVOperation::Get { key } => {
                        KVResponse::Get { key, value: None } // Always return None for now
                    }
                    KVOperation::Put { key, .. } => {
                        KVResponse::Put { key }
                    }
                    KVOperation::Delete { key } => {
                        KVResponse::Delete { key }
                    }
                    KVOperation::List => {
                        KVResponse::List { keys: vec![] } // Always return empty list for now
                    }
                }
            }
            Err(e) => {
                log::error!("TCP transport: failed to deserialize client request: {}", e);
                KVResponse::Error { message: "Invalid request format".to_string() }
            }
        };
        
        // Serialize the response
        let response_bytes = match Self::serialize_kv_response(&response) {
            Ok(bytes) => bytes,
            Err(e) => {
                log::error!("TCP transport: failed to serialize response: {}", e);
                return Err(std::io::Error::new(std::io::ErrorKind::Other, "Serialization failed"));
            }
        };
        
        // Send response header (4 bytes for length)
        let response_len = response_bytes.len() as u32;
        let mut writer = BufWriter::new(stream);
        writer.write_all(&response_len.to_be_bytes())?;
        
        // Send response data
        writer.write_all(&response_bytes)?;
        writer.flush()?;
        
        log::debug!("TCP transport: sent {} byte immediate response to client", response_bytes.len());
        
        Ok(())
    }
    
    /// Process client request data using actual KV store operations
    pub fn process_client_request_with_store(
        request_data: &[u8], 
        stream: &mut TcpStream, 
        kv_store: &mut crate::kv::InMemoryKVStore,
        _node_id: NodeId
    ) -> std::result::Result<(), std::io::Error> {
        use crate::kv::{KVOperation, KVResponse};
        
        log::debug!("TCP transport: processing client request data with actual KV store ({} bytes)", request_data.len());
        
        // Deserialize the KV operation
        let operation = match KVOperation::from_bytes(request_data) {
            Ok(op) => op,
            Err(e) => {
                log::error!("TCP transport: failed to deserialize client operation: {}", e);
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid operation"));
            }
        };
        
        log::debug!("TCP transport: processing client operation with real KV store: {:?}", operation);
        
        // Process the operation using the actual KV store
        let response = match operation {
            KVOperation::Get { key } => {
                log::debug!("Processing GET operation for key: {}", key);
                let value = kv_store.get(&key);
                KVResponse::Get { key, value }
            }
            KVOperation::Put { key, value } => {
                log::debug!("Processing PUT operation for key: {}", key);
                kv_store.put(key.clone(), value);
                log::debug!("PUT operation completed for key: {}", key);
                KVResponse::Put { key }
            }
            KVOperation::Delete { key } => {
                log::debug!("Processing DELETE operation for key: {}", key);
                let _existed = kv_store.delete(&key);
                log::debug!("DELETE operation completed for key: {}", key);
                KVResponse::Delete { key }
            }
            KVOperation::List => {
                log::debug!("Processing LIST operation");
                let keys = kv_store.list_keys();
                log::debug!("LIST operation completed, found {} keys", keys.len());
                KVResponse::List { keys }
            }
        };
        
        // Serialize the response
        let response_bytes = match Self::serialize_kv_response(&response) {
            Ok(bytes) => bytes,
            Err(e) => {
                log::error!("TCP transport: failed to serialize response: {}", e);
                return Err(std::io::Error::new(std::io::ErrorKind::Other, "Serialization failed"));
            }
        };
        
        // Send response header (4 bytes for length)
        let response_len = response_bytes.len() as u32;
        let mut writer = BufWriter::new(stream);
        writer.write_all(&response_len.to_be_bytes())?;
        
        // Send response data
        writer.write_all(&response_bytes)?;
        writer.flush()?;
        
        log::debug!("TCP transport: sent {} byte response to client using actual KV store", response_bytes.len());
        
        Ok(())
    }
    
    
    
    /// Serialize a KV response to bytes (copied from main.rs)
    fn serialize_kv_response(response: &crate::kv::KVResponse) -> Result<Vec<u8>> {
        use crate::kv::KVResponse;
        
        match response {
            KVResponse::Get { key, value } => {
                let mut bytes = vec![0u8]; // Response type: Get
                let key_len = key.len() as u32;
                bytes.extend_from_slice(&key_len.to_be_bytes());
                bytes.extend_from_slice(key.as_bytes());
                
                match value {
                    Some(val) => {
                        bytes.push(1u8); // Has value
                        bytes.extend_from_slice(val);
                    }
                    None => {
                        bytes.push(0u8); // No value
                    }
                }
                Ok(bytes)
            }
            KVResponse::Put { key } => {
                let mut bytes = vec![1u8]; // Response type: Put
                let key_len = key.len() as u32;
                bytes.extend_from_slice(&key_len.to_be_bytes());
                bytes.extend_from_slice(key.as_bytes());
                Ok(bytes)
            }
            KVResponse::Delete { key } => {
                let mut bytes = vec![2u8]; // Response type: Delete
                let key_len = key.len() as u32;
                bytes.extend_from_slice(&key_len.to_be_bytes());
                bytes.extend_from_slice(key.as_bytes());
                Ok(bytes)
            }
            KVResponse::List { keys } => {
                let mut bytes = vec![4u8]; // Response type: List
                let key_count = keys.len() as u32;
                bytes.extend_from_slice(&key_count.to_be_bytes());
                
                for key in keys {
                    let key_len = key.len() as u32;
                    bytes.extend_from_slice(&key_len.to_be_bytes());
                    bytes.extend_from_slice(key.as_bytes());
                }
                Ok(bytes)
            }
            KVResponse::Error { message } => {
                let mut bytes = vec![3u8]; // Response type: Error
                bytes.extend_from_slice(message.as_bytes());
                Ok(bytes)
            }
        }
    }
}

impl NetworkTransport for TcpTransport {
    fn send_message(&self, to: NodeId, message: Vec<u8>) -> Result<()> {
        log::debug!("TCP transport: attempting to send {} byte message to node {}", message.len(), to);
        
        // Get target address from cluster configuration
        if let Some(node_address) = self.config.cluster_addresses.get(&to) {
            log::debug!("TCP transport: sending message to node {} at address {}", to, node_address.socket_addr());
            
            // Connect to target node
            let addr: SocketAddr = node_address.socket_addr().parse()
                .map_err(|e| Error::Network(format!("Invalid address '{}': {}", node_address.socket_addr(), e)))?;
            
            let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
                .map_err(|e| Error::Network(format!("Failed to connect to {}: {}", addr, e)))?;
            
            let mut writer = BufWriter::new(stream);
            
            // Prepare framed message: 4 bytes length + 8 bytes sender node_id + message data
            // The length includes both the node_id and the message data
            let total_payload_len = (8 + message.len()) as u32;
            let mut framed_message = Vec::with_capacity(4 + 8 + message.len());
            
            // Add total payload length (4 bytes) - includes node_id + message data
            framed_message.extend_from_slice(&total_payload_len.to_be_bytes());
            
            // Add sender node_id (8 bytes)
            framed_message.extend_from_slice(&self.node_id.to_be_bytes());
            
            // Add message data
            framed_message.extend_from_slice(&message);
            
            // Send framed message
            writer.write_all(&framed_message)
                .map_err(|e| Error::Network(format!("Failed to send message: {}", e)))?;
            
            // Ensure message is sent immediately
            writer.flush()
                .map_err(|e| Error::Network(format!("Failed to flush message: {}", e)))?;
            
            log::debug!("TCP transport: successfully sent {} byte framed message to node {}", message.len(), to);
            Ok(())
        } else {
            log::error!("TCP transport: node {} address not found in cluster configuration", to);
            Err(Error::Network(format!("Node {} address not found", to)))
        }
    }
    
    fn receive_messages(&self) -> Vec<(NodeId, Vec<u8>)> {
        // Get messages from the incoming queue
        let mut incoming = self.incoming_messages.lock().unwrap();
        let messages = incoming.clone();
        incoming.clear();
        
        if !messages.is_empty() {
            log::debug!("TCP transport: delivering {} incoming messages", messages.len());
        }
        
        messages
    }
    
    fn is_connected(&self, node_id: NodeId) -> bool {
        let connections = self.connections.lock().unwrap();
        connections.contains_key(&node_id)
    }
    
    fn connected_nodes(&self) -> Vec<NodeId> {
        let connections = self.connections.lock().unwrap();
        connections.keys().cloned().collect()
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Mock network transport for testing
#[derive(Debug, Clone)]
pub struct MockTransport {
    /// Node ID of this transport
    node_id: NodeId,
    /// Messages to be received
    incoming_messages: Arc<Mutex<Vec<(NodeId, Vec<u8>)>>>,
    /// Messages that were sent
    sent_messages: Arc<Mutex<Vec<(NodeId, Vec<u8>)>>>,
    /// Connected nodes
    connected_nodes: Arc<Mutex<Vec<NodeId>>>,
    /// Whether this transport is isolated (for partition simulation)
    isolated: Arc<Mutex<bool>>,
    /// Whether this transport has failed (for failure simulation)
    failed: Arc<Mutex<bool>>,
}

impl MockTransport {
    /// Create a new mock transport
    pub fn new(node_id: NodeId) -> Self {
        Self {
            node_id,
            incoming_messages: Arc::new(Mutex::new(Vec::new())),
            sent_messages: Arc::new(Mutex::new(Vec::new())),
            connected_nodes: Arc::new(Mutex::new(Vec::new())),
            isolated: Arc::new(Mutex::new(false)),
            failed: Arc::new(Mutex::new(false)),
        }
    }
    
    /// Add a message to be received
    pub fn add_incoming_message(&self, from: NodeId, message: Vec<u8>) {
        self.incoming_messages.lock().unwrap().push((from, message));
    }
    
    /// Get all sent messages
    pub fn get_sent_messages(&self) -> Vec<(NodeId, Vec<u8>)> {
        self.sent_messages.lock().unwrap().clone()
    }
    
    /// Clear sent messages
    pub fn clear_sent_messages(&self) {
        self.sent_messages.lock().unwrap().clear();
    }
    
    /// Set connected nodes
    pub fn set_connected_nodes(&self, nodes: Vec<NodeId>) {
        *self.connected_nodes.lock().unwrap() = nodes;
    }
    
    /// Set isolation status (for network partition simulation)
    pub fn set_isolated(&self, isolated: bool) {
        *self.isolated.lock().unwrap() = isolated;
    }
    
    /// Check if this transport is isolated
    pub fn is_isolated(&self) -> bool {
        *self.isolated.lock().unwrap()
    }
    
    /// Set failure status (for node failure simulation)
    pub fn set_failed(&self, failed: bool) {
        *self.failed.lock().unwrap() = failed;
    }
    
    /// Check if this transport has failed
    pub fn is_failed(&self) -> bool {
        *self.failed.lock().unwrap()
    }
    
    /// Get pending messages (for integration testing)
    pub fn get_pending_messages(&self) -> Vec<(NodeId, Vec<u8>)> {
        let messages = self.sent_messages.lock().unwrap().clone();
        self.sent_messages.lock().unwrap().clear();
        messages
    }
}

impl NetworkTransport for MockTransport {
    fn send_message(&self, to: NodeId, message: Vec<u8>) -> Result<()> {
        self.sent_messages.lock().unwrap().push((to, message));
        Ok(())
    }
    
    fn receive_messages(&self) -> Vec<(NodeId, Vec<u8>)> {
        let mut messages = self.incoming_messages.lock().unwrap();
        let result = messages.clone();
        messages.clear();
        result
    }
    
    fn is_connected(&self, node_id: NodeId) -> bool {
        self.connected_nodes.lock().unwrap().contains(&node_id)
    }
    
    fn connected_nodes(&self) -> Vec<NodeId> {
        self.connected_nodes.lock().unwrap().clone()
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_transport() {
        let transport = MockTransport::new(1);
        
        // Test sending messages
        transport.send_message(2, b"hello".to_vec()).unwrap();
        transport.send_message(3, b"world".to_vec()).unwrap();
        
        let sent = transport.get_sent_messages();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0], (2, b"hello".to_vec()));
        assert_eq!(sent[1], (3, b"world".to_vec()));
        
        // Test receiving messages
        transport.add_incoming_message(2, b"response".to_vec());
        let received = transport.receive_messages();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0], (2, b"response".to_vec()));
        
        // Test connection status
        assert!(!transport.is_connected(2));
        transport.set_connected_nodes(vec![2, 3]);
        assert!(transport.is_connected(2));
        assert!(transport.is_connected(3));
        assert!(!transport.is_connected(4));
        
        let connected = transport.connected_nodes();
        assert_eq!(connected, vec![2, 3]);
    }
}
