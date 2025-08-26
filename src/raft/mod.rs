//! Raft consensus algorithm implementation
//! 
//! This module contains the core Raft consensus algorithm implementation,
//! including node state management, leader election, and log replication.

pub mod node;
pub mod state;
pub mod log;
pub mod messages;

pub use node::{RaftNode, MockRaftNode};
pub use state::RaftState;
pub use log::RaftLog;
pub use messages::{RaftMessage, AppendEntriesRequest, AppendEntriesResponse, RequestVoteRequest, RequestVoteResponse};

use crate::{Result, NodeId, Term, LogIndex};

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
    /// Election timeout range in milliseconds
    pub election_timeout_ms: (u64, u64),
    /// Heartbeat interval in milliseconds
    pub heartbeat_interval_ms: u64,
}

impl Default for RaftConfig {
    fn default() -> Self {
        Self {
            node_id: 0,
            cluster_nodes: vec![0],
            election_timeout_ms: (150, 300),
            heartbeat_interval_ms: 50,
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
        assert_eq!(config.election_timeout_ms, (150, 300));
        assert_eq!(config.heartbeat_interval_ms, 50);
    }
}
