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
use std::io::{Read, Write};
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
                log::info!("TCP transport: started listener thread for node {}", node_id);
                
                loop {
                    match listener_clone.accept() {
                        Ok((mut stream, addr)) => {
                            log::debug!("TCP transport: accepted connection from {}", addr);
                            
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
    
    /// Handle an incoming TCP connection
    fn handle_connection(
        stream: &mut TcpStream, 
        incoming_messages: Arc<Mutex<Vec<(NodeId, Vec<u8>)>>>,
        node_id: NodeId
    ) {
        log::debug!("TCP transport: handling connection for node {}", node_id);
        
        let mut buffer = [0; 4096];
        loop {
            match stream.read(&mut buffer) {
                Ok(0) => {
                    log::debug!("TCP transport: connection closed");
                    break;
                }
                Ok(n) => {
                    log::trace!("TCP transport: received {} bytes", n);
                    
                    // Parse message (simple format: 8 bytes for sender node_id + message data)
                    if n >= 8 {
                        let sender_bytes = &buffer[0..8];
                        let sender_id = u64::from_be_bytes([
                            sender_bytes[0], sender_bytes[1], sender_bytes[2], sender_bytes[3],
                            sender_bytes[4], sender_bytes[5], sender_bytes[6], sender_bytes[7],
                        ]);
                        
                        let message_data = buffer[8..n].to_vec();
                        
                        log::debug!("TCP transport: received message from node {} ({} bytes)", 
                                   sender_id, message_data.len());
                        
                        // Add to incoming message queue
                        incoming_messages.lock().unwrap().push((sender_id, message_data));
                    } else {
                        log::warn!("TCP transport: received message too short ({} bytes)", n);
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    log::error!("TCP transport: error reading from connection: {}", e);
                    break;
                }
            }
        }
    }
}

impl NetworkTransport for TcpTransport {
    fn send_message(&self, to: NodeId, message: Vec<u8>) -> Result<()> {
        log::debug!("TCP transport: attempting to send {} byte message to node {}", message.len(), to);
        
        // Get target address from cluster configuration
        if let Some(node_address) = self.config.cluster_addresses.get(&to) {
            log::trace!("TCP transport: sending message to node {} at address {}", to, node_address.socket_addr());
            
            // Connect to target node
            let addr: SocketAddr = node_address.socket_addr().parse()
                .map_err(|e| Error::Network(format!("Invalid address '{}': {}", node_address.socket_addr(), e)))?;
            
            let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
                .map_err(|e| Error::Network(format!("Failed to connect to {}: {}", addr, e)))?;
            
            // Prepare message with sender node_id prefix (8 bytes + message data)
            let mut full_message = Vec::with_capacity(8 + message.len());
            full_message.extend_from_slice(&self.node_id.to_be_bytes());
            full_message.extend_from_slice(&message);
            
            // Send message
            stream.write_all(&full_message)
                .map_err(|e| Error::Network(format!("Failed to send message: {}", e)))?;
            
            log::info!("TCP transport: successfully sent {} byte message to node {}", message.len(), to);
            Ok(())
        } else {
            log::error!("TCP transport: node {} address not found in cluster configuration", to);
            Err(Error::Network(format!("Node {} address not found", to)))
        }
    }
    
    fn receive_messages(&self) -> Vec<(NodeId, Vec<u8>)> {
        log::trace!("TCP transport: checking for incoming messages");
        
        // Get messages from the incoming queue
        let mut incoming = self.incoming_messages.lock().unwrap();
        let messages = incoming.clone();
        incoming.clear();
        
        if !messages.is_empty() {
            log::debug!("TCP transport: delivering {} incoming messages", messages.len());
            for (from, msg) in &messages {
                log::trace!("TCP transport: delivering {} byte message from node {}", msg.len(), from);
            }
        }
        
        messages
    }
    
    fn is_connected(&self, node_id: NodeId) -> bool {
        let connections = self.connections.lock().unwrap();
        let connected = connections.contains_key(&node_id);
        log::trace!("TCP transport: connection status for node {}: {}", node_id, connected);
        connected
    }
    
    fn connected_nodes(&self) -> Vec<NodeId> {
        let connections = self.connections.lock().unwrap();
        let connected: Vec<NodeId> = connections.keys().cloned().collect();
        
        log::debug!("TCP transport: {} nodes currently connected: {:?}", connected.len(), connected);
        connected
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
        log::debug!("Mock transport (node {}): sending {} byte message to node {}", 
                   self.node_id, message.len(), to);
        log::trace!("Mock transport (node {}): message content: {:?}", self.node_id, message);
        
        self.sent_messages.lock().unwrap().push((to, message.clone()));
        
        log::info!("Mock transport (node {}): successfully queued message for node {}", 
                  self.node_id, to);
        Ok(())
    }
    
    fn receive_messages(&self) -> Vec<(NodeId, Vec<u8>)> {
        let mut messages = self.incoming_messages.lock().unwrap();
        let result = messages.clone();
        messages.clear();
        
        if !result.is_empty() {
            log::debug!("Mock transport (node {}): delivering {} incoming messages", 
                       self.node_id, result.len());
            for (from, msg) in &result {
                log::trace!("Mock transport (node {}): delivering {} byte message from node {}", 
                           self.node_id, msg.len(), from);
            }
        } else {
            log::trace!("Mock transport (node {}): no incoming messages", self.node_id);
        }
        
        result
    }
    
    fn is_connected(&self, node_id: NodeId) -> bool {
        let connected = self.connected_nodes.lock().unwrap().contains(&node_id);
        log::trace!("Mock transport (node {}): connection status for node {}: {}", 
                   self.node_id, node_id, connected);
        connected
    }
    
    fn connected_nodes(&self) -> Vec<NodeId> {
        let connected = self.connected_nodes.lock().unwrap().clone();
        log::debug!("Mock transport (node {}): {} nodes connected: {:?}", 
                   self.node_id, connected.len(), connected);
        connected
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
