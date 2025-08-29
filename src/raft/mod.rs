//! Raft consensus algorithm implementation
//! 
//! This module contains the core Raft consensus algorithm implementation,
//! including node state management, leader election, and log replication.

pub mod node;
pub mod state;
pub mod log;
pub mod messages;
pub mod client_tracker;
pub mod client_response_tests;

pub use node::{RaftNode, MockRaftNode};
pub use state::RaftState;
pub use log::RaftLog;
pub use messages::{
    RaftMessage, AppendEntriesRequest, AppendEntriesResponse, RequestVoteRequest, RequestVoteResponse,
    ClientRequest, ClientResponse, LeaderRedirect,
};
pub use client_tracker::{ClientRequestTracker, PendingRequest, RequestId};

use crate::{NodeId, timing::TimingConfig};

/// Raft node states
#[derive(Debug, Clone, PartialEq)]
pub enum NodeState {
    /// Follower state - accepts entries from leader
    Follower,
    /// Candidate state - requesting votes for leadership
    Candidate,
    /// Leader state - replicating entries to followers
    Leader,
}

/// Raft configuration parameters
#[derive(Debug, Clone)]
pub struct RaftConfig {
    /// Unique identifier for this node
    pub node_id: NodeId,
    /// List of all nodes in the cluster
    pub cluster_nodes: Vec<NodeId>,
    /// Timing configuration for this node
    pub timing: TimingConfig,
}

impl RaftConfig {
    /// Create a new RaftConfig with the specified timing configuration
    pub fn new(node_id: NodeId, cluster_nodes: Vec<NodeId>, timing: TimingConfig) -> Self {
        Self {
            node_id,
            cluster_nodes,
            timing,
        }
    }
    
    /// Create a new RaftConfig with fast timing (production mode)
    pub fn fast(node_id: NodeId, cluster_nodes: Vec<NodeId>) -> Self {
        Self::new(node_id, cluster_nodes, TimingConfig::fast())
    }
    
    /// Create a new RaftConfig with debug timing
    pub fn debug(node_id: NodeId, cluster_nodes: Vec<NodeId>) -> Self {
        Self::new(node_id, cluster_nodes, TimingConfig::debug())
    }
    
    /// Create a new RaftConfig with demo timing
    pub fn demo(node_id: NodeId, cluster_nodes: Vec<NodeId>) -> Self {
        Self::new(node_id, cluster_nodes, TimingConfig::demo())
    }
    
    /// Get election timeout range (for backward compatibility)
    pub fn election_timeout_ms(&self) -> (u64, u64) {
        self.timing.election_timeout_range()
    }
    
    /// Get heartbeat interval (for backward compatibility)
    pub fn heartbeat_interval_ms(&self) -> u64 {
        self.timing.heartbeat_interval_ms
    }
}

impl Default for RaftConfig {
    fn default() -> Self {
        Self {
            node_id: 0,
            cluster_nodes: vec![0],
            timing: TimingConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_state_equality() {
        assert_eq!(NodeState::Follower, NodeState::Follower);
        assert_ne!(NodeState::Follower, NodeState::Leader);
    }

    #[test]
    fn test_raft_config_default() {
        let config = RaftConfig::default();
        assert_eq!(config.node_id, 0);
        assert_eq!(config.cluster_nodes, vec![0]);
        assert_eq!(config.election_timeout_ms(), (150, 300));
        assert_eq!(config.heartbeat_interval_ms(), 50);
    }

    #[test]
    fn test_raft_config_constructors() {
        let cluster_nodes = vec![0, 1, 2];
        
        let fast_config = RaftConfig::fast(1, cluster_nodes.clone());
        assert_eq!(fast_config.node_id, 1);
        assert_eq!(fast_config.cluster_nodes, cluster_nodes);
        assert_eq!(fast_config.timing, TimingConfig::fast());
        
        let debug_config = RaftConfig::debug(2, cluster_nodes.clone());
        assert_eq!(debug_config.node_id, 2);
        assert_eq!(debug_config.timing, TimingConfig::debug());
        
        let demo_config = RaftConfig::demo(3, cluster_nodes.clone());
        assert_eq!(demo_config.node_id, 3);
        assert_eq!(demo_config.timing, TimingConfig::demo());
    }

    #[test]
    fn test_raft_config_backward_compatibility() {
        let config = RaftConfig::fast(0, vec![0, 1, 2]);
        
        // Test backward compatibility methods
        let (min, max) = config.election_timeout_ms();
        assert_eq!(min, 150);
        assert_eq!(max, 300);
        
        assert_eq!(config.heartbeat_interval_ms(), 50);
    }
}
