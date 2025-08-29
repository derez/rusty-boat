//! kvapp-c: A Raft-based distributed key-value store library
//! 
//! This library implements the Raft consensus algorithm to provide a distributed
//! key-value store that maintains consistency across multiple nodes.
//! 
//! # Architecture
//! 
//! The system follows an event-driven architecture with dependency injection:
//! - **raft**: Core Raft consensus algorithm implementation
//! - **storage**: Persistent storage abstractions and implementations
//! - **network**: Network communication and message bus
//! - **kv**: Key-value store operations and client interface
//! 
//! # Design Principles
//! 
//! - Synchronous design (no async runtime)
//! - Trait-based interfaces for dependency injection
//! - Event-driven communication between components
//! - Comprehensive testing with mock implementations
//! 
//! # Usage
//! 
//! ```rust
//! use kvapp_c::{RaftNode, RaftConfig, storage, network, kv, Result};
//! 
//! fn example() -> Result<()> {
//!     // Create storage backends
//!     let state_storage = Box::new(storage::InMemoryStateStorage::new());
//!     let log_storage = Box::new(storage::InMemoryLogStorage::new());
//!     
//!     // Create network transport
//!     let transport = Box::new(network::MockTransport::new(0));
//!     
//!     // Create Raft configuration
//!     let config = RaftConfig {
//!         node_id: 0,
//!         cluster_nodes: vec![0, 1, 2],
//!         election_timeout_ms: (150, 300),
//!         heartbeat_interval_ms: 50,
//!     };
//!     
//!     // Initialize Raft node
//!     let _raft_node = RaftNode::with_dependencies(
//!         config,
//!         state_storage,
//!         log_storage,
//!         transport,
//!     )?;
//!     
//!     Ok(())
//! }
//! ```

pub mod raft;
pub mod storage;
pub mod network;
pub mod kv;

#[cfg(test)]
pub mod tests;

// Re-export commonly used types for convenience
pub use raft::{RaftNode, RaftState, RaftConfig, RaftMessage};
pub use storage::{LogStorage, StateStorage, KVStorage};
pub use network::{NetworkTransport, MessageBus, NetworkConfig, NodeAddress};
pub use kv::{KVStore, KVClient, KVOperation, KVResponse};

/// Common result type used throughout the library
pub type Result<T> = std::result::Result<T, Error>;

/// Main error type for the kvapp-c library
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// Storage-related errors
    Storage(String),
    /// Network-related errors
    Network(String),
    /// Raft consensus errors
    Raft(String),
    /// Key-value operation errors
    KeyValue(String),
    /// I/O errors
    Io(String),
    /// Serialization/deserialization errors
    Serialization(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Storage(msg) => write!(f, "Storage error: {}", msg),
            Error::Network(msg) => write!(f, "Network error: {}", msg),
            Error::Raft(msg) => write!(f, "Raft error: {}", msg),
            Error::KeyValue(msg) => write!(f, "Key-value error: {}", msg),
            Error::Io(msg) => write!(f, "I/O error: {}", msg),
            Error::Serialization(msg) => write!(f, "Serialization error: {}", msg),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err.to_string())
    }
}

/// Node identifier type
/// 
/// Used to uniquely identify nodes in a Raft cluster.
/// Typically assigned sequentially starting from 0.
pub type NodeId = u64;

/// Raft term type
/// 
/// Represents the current term in the Raft consensus algorithm.
/// Terms are used to detect stale information and ensure consistency.
pub type Term = u64;

/// Log index type
/// 
/// Represents the index of entries in the Raft log.
/// Log indices start from 1 (index 0 is reserved).
pub type LogIndex = u64;

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let storage_err = Error::Storage("test error".to_string());
        assert_eq!(storage_err.to_string(), "Storage error: test error");
        
        let network_err = Error::Network("connection failed".to_string());
        assert_eq!(network_err.to_string(), "Network error: connection failed");
        
        let raft_err = Error::Raft("election failed".to_string());
        assert_eq!(raft_err.to_string(), "Raft error: election failed");
        
        let kv_err = Error::KeyValue("key not found".to_string());
        assert_eq!(kv_err.to_string(), "Key-value error: key not found");
        
        let io_err = Error::Io("file not found".to_string());
        assert_eq!(io_err.to_string(), "I/O error: file not found");
        
        let serialization_err = Error::Serialization("invalid format".to_string());
        assert_eq!(serialization_err.to_string(), "Serialization error: invalid format");
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let kvapp_err: Error = io_err.into();
        
        match kvapp_err {
            Error::Io(msg) => assert!(msg.contains("file not found")),
            _ => panic!("Expected Io error"),
        }
    }

    #[test]
    fn test_error_equality() {
        let err1 = Error::Storage("test".to_string());
        let err2 = Error::Storage("test".to_string());
        let err3 = Error::Storage("different".to_string());
        
        assert_eq!(err1, err2);
        assert_ne!(err1, err3);
    }

    #[test]
    fn test_error_clone() {
        let original = Error::Network("connection failed".to_string());
        let cloned = original.clone();
        
        assert_eq!(original, cloned);
    }
}
