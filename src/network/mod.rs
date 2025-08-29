//! Network communication and event system
//! 
//! This module provides network transport for inter-node communication
//! and an event-driven message bus for loose coupling between components.

pub mod transport;
pub mod message_bus;

pub use transport::{NetworkTransport, TcpTransport, MockTransport};
pub use message_bus::{MessageBus, MockEventHandler, EventCounterHandler};

use crate::{Result, NodeId};
use std::collections::HashMap;

/// Network address for a node with dual-port support
#[derive(Debug, Clone, PartialEq)]
pub struct NodeAddress {
    /// Node identifier
    pub node_id: NodeId,
    /// Host address (IP or hostname)
    pub host: String,
    /// Port number for Raft inter-node communication
    pub raft_port: u16,
    /// Port number for client communication (raft_port + offset)
    pub client_port: u16,
}

impl NodeAddress {
    /// Create a new node address with default client port offset (+1000)
    pub fn new(node_id: NodeId, host: String, raft_port: u16) -> Self {
        Self::with_client_port_offset(node_id, host, raft_port, 1000)
    }
    
    /// Create a new node address with custom client port offset
    pub fn with_client_port_offset(node_id: NodeId, host: String, raft_port: u16, client_port_offset: u16) -> Self {
        Self { 
            node_id, 
            host, 
            raft_port,
            client_port: raft_port + client_port_offset,
        }
    }
    
    /// Get the Raft socket address string
    pub fn raft_socket_addr(&self) -> String {
        format!("{}:{}", self.host, self.raft_port)
    }
    
    /// Get the client socket address string
    pub fn client_socket_addr(&self) -> String {
        format!("{}:{}", self.host, self.client_port)
    }
    
    /// Get the socket address string (backward compatibility - returns Raft port)
    pub fn socket_addr(&self) -> String {
        self.raft_socket_addr()
    }
}

/// Network configuration for the cluster
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    /// This node's address
    pub local_address: NodeAddress,
    /// Addresses of all nodes in the cluster
    pub cluster_addresses: HashMap<NodeId, NodeAddress>,
    /// Connection timeout in milliseconds
    pub connection_timeout_ms: u64,
    /// Maximum message size in bytes
    pub max_message_size: usize,
}

impl NetworkConfig {
    /// Create a new network configuration
    pub fn new(local_address: NodeAddress) -> Self {
        let mut cluster_addresses = HashMap::new();
        cluster_addresses.insert(local_address.node_id, local_address.clone());
        
        Self {
            local_address,
            cluster_addresses,
            connection_timeout_ms: 5000,
            max_message_size: 1024 * 1024, // 1MB
        }
    }
    
    /// Add a node address to the cluster
    pub fn add_node(&mut self, address: NodeAddress) {
        self.cluster_addresses.insert(address.node_id, address);
    }
    
    /// Get address for a specific node
    pub fn get_node_address(&self, node_id: NodeId) -> Option<&NodeAddress> {
        self.cluster_addresses.get(&node_id)
    }
}

/// Event types for the message bus
#[derive(Debug, Clone)]
pub enum Event {
    /// Raft-related events
    Raft(RaftEvent),
    /// Client request events
    Client(ClientEvent),
    /// Network message events
    Network(NetworkEvent),
    /// Timer events
    Timer(TimerEvent),
}

/// Raft consensus events
#[derive(Debug, Clone)]
pub enum RaftEvent {
    /// Election timeout occurred
    ElectionTimeout,
    /// Heartbeat timeout occurred
    HeartbeatTimeout,
    /// State transition occurred
    StateTransition { from: String, to: String },
    /// Log entry committed
    LogCommitted { index: u64 },
}

/// Client request events
#[derive(Debug, Clone)]
pub enum ClientEvent {
    /// Get request
    Get { key: String },
    /// Put request
    Put { key: String, value: Vec<u8> },
    /// Delete request
    Delete { key: String },
    /// List request
    List,
}

/// Network message events
#[derive(Debug, Clone)]
pub enum NetworkEvent {
    /// Message received from another node
    MessageReceived { from: NodeId, data: Vec<u8> },
    /// Connection established
    ConnectionEstablished { node_id: NodeId },
    /// Connection lost
    ConnectionLost { node_id: NodeId },
}

/// Timer events
#[derive(Debug, Clone)]
pub enum TimerEvent {
    /// Election timer fired
    ElectionTimer,
    /// Heartbeat timer fired
    HeartbeatTimer,
}

/// Trait for handling events
pub trait EventHandler: Send + Sync {
    /// Handle an event
    fn handle_event(&mut self, event: Event) -> Result<()>;
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_address_creation() {
        let addr = NodeAddress::new(1, "localhost".to_string(), 8080);
        assert_eq!(addr.node_id, 1);
        assert_eq!(addr.host, "localhost");
        assert_eq!(addr.raft_port, 8080);
        assert_eq!(addr.client_port, 9080); // Default +1000 offset
        assert_eq!(addr.raft_socket_addr(), "localhost:8080");
        assert_eq!(addr.client_socket_addr(), "localhost:9080");
        assert_eq!(addr.socket_addr(), "localhost:8080"); // Backward compatibility
    }

    #[test]
    fn test_node_address_custom_offset() {
        let addr = NodeAddress::with_client_port_offset(1, "localhost".to_string(), 8080, 500);
        assert_eq!(addr.node_id, 1);
        assert_eq!(addr.host, "localhost");
        assert_eq!(addr.raft_port, 8080);
        assert_eq!(addr.client_port, 8580); // Custom +500 offset
        assert_eq!(addr.raft_socket_addr(), "localhost:8080");
        assert_eq!(addr.client_socket_addr(), "localhost:8580");
    }

    #[test]
    fn test_network_config_creation() {
        let addr = NodeAddress::new(1, "localhost".to_string(), 8080);
        let config = NetworkConfig::new(addr.clone());
        
        assert_eq!(config.local_address, addr);
        assert_eq!(config.cluster_addresses.len(), 1);
        assert!(config.cluster_addresses.contains_key(&1));
    }

    #[test]
    fn test_network_config_add_node() {
        let local_addr = NodeAddress::new(1, "localhost".to_string(), 8080);
        let mut config = NetworkConfig::new(local_addr);
        
        let remote_addr = NodeAddress::new(2, "remote".to_string(), 8081);
        config.add_node(remote_addr.clone());
        
        assert_eq!(config.cluster_addresses.len(), 2);
        assert_eq!(config.get_node_address(2), Some(&remote_addr));
    }
}
