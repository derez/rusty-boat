//! Network transport implementations
//! 
//! This module provides network transport for inter-node communication
//! with both TCP-based and mock implementations for testing.

use crate::{Result, Error, NodeId};
use super::{NodeAddress, NetworkConfig};

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
    /// Node ID of this transport
    node_id: NodeId,
    /// Whether the listener thread is running
    listener_running: Arc<Mutex<bool>>,
}

impl TcpTransport {
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
            let node_id = self.node_id;
            
            thread::spawn(move || {
                log::debug!("TCP transport: started listener thread for node {}", node_id);
                
                loop {
                    match listener_clone.accept() {
                        Ok((mut stream, addr)) => {
                            log::info!("TCP transport: accepted connection from {}", addr);
                            
                            // Handle connection in a separate thread
                            let incoming_messages_clone = Arc::clone(&incoming_messages);
                            thread::spawn(move || {
                                Self::handle_connection(&mut stream, incoming_messages_clone, node_id);
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
        node_id: NodeId
    ) {
        log::debug!("TCP transport: handling connection for node {}", node_id);
        
        // Set stream to blocking mode for easier message parsing
        stream.set_nonblocking(false).unwrap_or_else(|e| {
            log::error!("TCP transport: failed to set blocking mode: {}", e);
        });
        
        let mut reader = BufReader::new(stream);
        let mut message_buffer = Vec::new();
        
        loop {
            // Read message header: 4 bytes for message length + 8 bytes for sender node_id
            let mut header = [0u8; 12];
            match reader.read_exact(&mut header) {
                Ok(()) => {
                    // Parse message length (first 4 bytes)
                    let message_len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
                    
                    // Parse sender node_id (next 8 bytes)
                    let sender_id = u64::from_be_bytes([
                        header[4], header[5], header[6], header[7],
                        header[8], header[9], header[10], header[11],
                    ]);
                    
                    // Validate message length (prevent excessive memory allocation)
                    if message_len > 1024 * 1024 { // 1MB limit
                        log::warn!("TCP transport: message too large ({} bytes), closing connection", message_len);
                        break;
                    }
                    
                    // Read message data
                    message_buffer.clear();
                    message_buffer.resize(message_len, 0);
                    
                    match reader.read_exact(&mut message_buffer) {
                        Ok(()) => {
                            log::debug!("TCP transport: received complete message from node {} ({} bytes)",
                                       sender_id, message_len);
                            
                            // Add to incoming message queue
                            incoming_messages.lock().unwrap().push((sender_id, message_buffer.clone()));
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
                        log::error!("TCP transport: error reading message header: {}", e);
                    }
                    break;
                }
            }
        }
        
        log::debug!("TCP transport: connection handler exiting for node {}", node_id);
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
            let message_len = message.len() as u32;
            let mut framed_message = Vec::with_capacity(4 + 8 + message.len());
            
            // Add message length (4 bytes)
            framed_message.extend_from_slice(&message_len.to_be_bytes());
            
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
