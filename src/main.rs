//! kvapp-c: A Raft-based distributed key-value store
//! 
//! This application implements the Raft consensus algorithm to provide a distributed
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

pub mod raft;
pub mod storage;
pub mod network;
pub mod kv;

#[cfg(test)]
pub mod tests;

// Re-export commonly used types
pub use raft::{RaftNode, RaftState};
pub use storage::{LogStorage, StateStorage};
pub use network::{NetworkTransport, MessageBus};
pub use kv::{KVStore, KVClient};

/// Common result type used throughout the application
pub type Result<T> = std::result::Result<T, Error>;

/// Main error type for the kvapp-c application
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
pub type NodeId = u64;

/// Raft term type
pub type Term = u64;

/// Log index type
pub type LogIndex = u64;

fn main() {
    // Initialize logging
    env_logger::init();
    
    log::info!("Starting kvapp-c: Raft-based distributed key-value store");
    log::info!("Phase 1 implementation complete - all core infrastructure ready");
    log::info!("Ready for Phase 2: Consensus Implementation");
    
    // Application would continue with actual Raft node initialization here
    log::info!("Application startup complete");
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let storage_err = Error::Storage("test error".to_string());
        assert_eq!(storage_err.to_string(), "Storage error: test error");
        
        let network_err = Error::Network("connection failed".to_string());
        assert_eq!(network_err.to_string(), "Network error: connection failed");
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
}
