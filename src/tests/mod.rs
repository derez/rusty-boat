//! Integration tests module
//! 
//! This module contains integration tests for multi-node Raft cluster scenarios.

pub mod integration;
pub mod tcp_transport;
// pub mod client_consistency; // Temporarily disabled due to compilation errors
pub mod client_consistency_simple;
