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

/// TCP-based network transport implementation
pub struct TcpTransport {
    /// Network configuration
    config: NetworkConfig,
    /// TCP listener for incoming connections
    listener: Option<TcpListener>,
    /// Connected nodes with their TCP streams
    connections: Arc<Mutex<HashMap<NodeId, TcpStream>>>,
    /// Incoming message queue
    incoming_messages: Arc<Mutex<Vec<(NodeId, Vec<u8>)>>>,
    /// Client request queue
    client_requests: Arc<Mutex<Vec<(std::net::TcpStream, Vec<u8>)>>>,
    /// Node ID of this transport
    node_id: NodeId,
    /// Whether the listener thread is running
    listener_running: Arc<Mutex<bool>>,
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
    
    /// Create a new TCP transport
    pub fn new(config: NetworkConfig, node_id: NodeId, bind_address: &str) -> Result<Self> {
        // Parse bind address
        let bind_addr: SocketAddr = bind_address.parse()
            .map_err(|e| Error::Network(format!("Invalid bind address '{}': {}", bind_address, e)))?;
        
        // Create TCP listener
        let listener = TcpListener::bind(bind_addr)
            .map_err(|e| Error::Network(format!("Failed to bind to {}: {}", bind_addr, e)))?;
        
        // Set non-blocking mode for the listener
        listener.set_nonblocking(true)
            .map_err(|e| Error::Network(format!("Failed to set non-blocking mode: {}", e)))?;
        
        log::info!("TCP transport: listening on {} for node {}", bind_addr, node_id);
        
        Ok(Self {
            config,
            listener: Some(listener),
            connections: Arc::new(Mutex::new(HashMap::new())),
            incoming_messages: Arc::new(Mutex::new(Vec::new())),
            client_requests: Arc::new(Mutex::new(Vec::new())),
            node_id,
            listener_running: Arc::new(Mutex::new(false)),
        })
    }
    
    /// Start accepting incoming connections in a background thread
    pub fn start_listener(&self) -> Result<()> {
        if let Some(ref listener) = self.listener {
            let listener_clone = listener.try_clone()
                .map_err(|e| Error::Network(format!("Failed to clone listener: {}", e)))?;
            let incoming_messages = Arc::clone(&self.incoming_messages);
            let client_requests = Arc::clone(&self.client_requests);
            let node_id = self.node_id;
            
            thread::spawn(move || {
                log::debug!("TCP transport: started listener thread for node {}", node_id);
                
                loop {
                    match listener_clone.accept() {
                        Ok((mut stream, addr)) => {
                            log::info!("TCP transport: accepted connection from {}", addr);
                            
                            // Handle connection in a separate thread
                            let incoming_messages_clone = Arc::clone(&incoming_messages);
                            let client_requests_clone = Arc::clone(&client_requests);
                            thread::spawn(move || {
                                Self::handle_connection(&mut stream, incoming_messages_clone, client_requests_clone, node_id);
                            });
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            // No pending connections, sleep briefly
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(e) => {
                            log::error!("TCP transport: error accepting connection: {}", e);
                            thread::sleep(Duration::from_millis(100));
                        }
                    }
                }
            });
        }
        
        Ok(())
    }
    
    /// Handle an incoming TCP connection with proper message framing
    fn handle_connection(
        stream: &mut TcpStream, 
        incoming_messages: Arc<Mutex<Vec<(NodeId, Vec<u8>)>>>,
        client_requests: Arc<Mutex<Vec<(TcpStream, Vec<u8>)>>>,
        node_id: NodeId
    ) {
        log::debug!("TCP transport: handling connection for node {}", node_id);
        
        // Set stream to blocking mode for easier message parsing
        stream.set_nonblocking(false).unwrap_or_else(|e| {
            log::error!("TCP transport: failed to set blocking mode: {}", e);
        });
        
        // Clone the stream for writing responses (needed for client requests)
        let write_stream = match stream.try_clone() {
            Ok(s) => s,
            Err(e) => {
                log::error!("TCP transport: failed to clone stream: {}", e);
                return;
            }
        };
        
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
                        log::warn!("TCP transport: message too large ({} bytes), closing connection", message_len);
                        break;
                    }
                    
                    // Read the complete message data
                    message_buffer.clear();
                    message_buffer.resize(message_len, 0);
                    
                    match reader.read_exact(&mut message_buffer) {
                        Ok(()) => {
                            log::debug!("TCP transport: received {} byte message", message_len);
                            
                            // Determine message type by checking if it starts with a valid node_id
                            // Raft messages from send_message() have format: node_id (8 bytes) + raft_message_data
                            // Client messages have format: kv_operation_data (starts with operation type byte)
                            
                            let is_raft_message = if message_buffer.len() >= 8 {
                                // Check if the first 8 bytes could be a valid node_id
                                let potential_node_id = u64::from_be_bytes([
                                    message_buffer[0], message_buffer[1], message_buffer[2], message_buffer[3],
                                    message_buffer[4], message_buffer[5], message_buffer[6], message_buffer[7]
                                ]);
                                
                                // Raft messages have reasonable node_id values (0-1000) and sufficient length
                                potential_node_id <= 1000 && message_buffer.len() > 8
                            } else {
                                false
                            };
                            
                            if is_raft_message {
                                // Handle as Raft message
                                let sender_node_id = u64::from_be_bytes([
                                    message_buffer[0], message_buffer[1], message_buffer[2], message_buffer[3],
                                    message_buffer[4], message_buffer[5], message_buffer[6], message_buffer[7]
                                ]);
                                
                                let raft_message_data = message_buffer[8..].to_vec();
                                
                                log::debug!("TCP transport: detected Raft message from node {} ({} bytes)", 
                                           sender_node_id, raft_message_data.len());
                                
                                // Add to incoming message queue
                                incoming_messages.lock().unwrap().push((sender_node_id, raft_message_data));
                            } else {
                                // Handle as client request
                                log::debug!("TCP transport: detected client request ({} bytes)", message_buffer.len());
                                
                                // Store the client request for processing by the server event loop
                                log::debug!("TCP transport: storing client request for server processing");
                                client_requests.lock().unwrap().push((write_stream.try_clone().unwrap(), message_buffer.clone()));
                                
                                // Client requests are one-shot, close connection after queuing
                                break;
                            }
                        }
                        Err(e) => {
                            log::error!("TCP transport: error reading message data: {}", e);
                            break;
                        }
                    }
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::UnexpectedEof {
                        log::info!("TCP transport: connection closed by peer");
                    } else {
                        log::error!("TCP transport: error reading message length: {}", e);
                    }
                    break;
                }
            }
        }
        
        log::debug!("TCP transport: connection handler exiting for node {}", node_id);
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
