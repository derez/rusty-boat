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

/// TCP-based network transport implementation
pub struct TcpTransport {
    /// Network configuration
    config: NetworkConfig,
    /// Connected nodes (stub implementation)
    connections: Arc<Mutex<HashMap<NodeId, bool>>>,
}

impl TcpTransport {
    /// Create a new TCP transport
    pub fn new(config: NetworkConfig) -> Result<Self> {
        Ok(Self {
            config,
            connections: Arc::new(Mutex::new(HashMap::new())),
        })
    }
}

impl NetworkTransport for TcpTransport {
    fn send_message(&self, to: NodeId, message: Vec<u8>) -> Result<()> {
        // Stub implementation - in real implementation would send over TCP
        if self.config.cluster_addresses.contains_key(&to) {
            Ok(())
        } else {
            Err(Error::Network(format!("Unknown node: {}", to)))
        }
    }
    
    fn receive_messages(&self) -> Vec<(NodeId, Vec<u8>)> {
        // Stub implementation - would receive from TCP sockets
        Vec::new()
    }
    
    fn is_connected(&self, node_id: NodeId) -> bool {
        self.connections.lock().unwrap().get(&node_id).copied().unwrap_or(false)
    }
    
    fn connected_nodes(&self) -> Vec<NodeId> {
        self.connections.lock().unwrap()
            .iter()
            .filter_map(|(id, connected)| if *connected { Some(*id) } else { None })
            .collect()
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
}

impl MockTransport {
    /// Create a new mock transport
    pub fn new(node_id: NodeId) -> Self {
        Self {
            node_id,
            incoming_messages: Arc::new(Mutex::new(Vec::new())),
            sent_messages: Arc::new(Mutex::new(Vec::new())),
            connected_nodes: Arc::new(Mutex::new(Vec::new())),
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
