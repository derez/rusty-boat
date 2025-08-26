//! Raft state management
//! 
//! This module manages the state of a Raft node including persistent state,
//! volatile state, and leader-specific state.

use crate::{Result, NodeId, Term, LogIndex};
use super::NodeState;

/// Raft state manager
#[derive(Debug, Clone)]
pub struct RaftState {
    /// Current node state
    node_state: NodeState,
    /// Current term
    current_term: Term,
    /// Candidate voted for in current term
    voted_for: Option<NodeId>,
    /// Index of highest log entry known to be committed
    commit_index: LogIndex,
    /// Index of highest log entry applied to state machine
    last_applied: LogIndex,
}

impl RaftState {
    /// Create a new Raft state
    pub fn new() -> Self {
        Self {
            node_state: NodeState::Follower,
            current_term: 0,
            voted_for: None,
            commit_index: 0,
            last_applied: 0,
        }
    }
    
    /// Get the current node state
    pub fn node_state(&self) -> &NodeState {
        &self.node_state
    }
    
    /// Set the node state
    pub fn set_node_state(&mut self, state: NodeState) {
        self.node_state = state;
    }
    
    /// Get the current term
    pub fn current_term(&self) -> Term {
        self.current_term
    }
    
    /// Set the current term
    pub fn set_current_term(&mut self, term: Term) {
        self.current_term = term;
    }
    
    /// Get the voted for candidate
    pub fn voted_for(&self) -> Option<NodeId> {
        self.voted_for
    }
    
    /// Set the voted for candidate
    pub fn set_voted_for(&mut self, candidate: Option<NodeId>) {
        self.voted_for = candidate;
    }
    
    /// Get the commit index
    pub fn commit_index(&self) -> LogIndex {
        self.commit_index
    }
    
    /// Set the commit index
    pub fn set_commit_index(&mut self, index: LogIndex) {
        self.commit_index = index;
    }
    
    /// Get the last applied index
    pub fn last_applied(&self) -> LogIndex {
        self.last_applied
    }
    
    /// Set the last applied index
    pub fn set_last_applied(&mut self, index: LogIndex) {
        self.last_applied = index;
    }
}

impl Default for RaftState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raft_state_creation() {
        let state = RaftState::new();
        
        assert!(matches!(state.node_state(), NodeState::Follower));
        assert_eq!(state.current_term(), 0);
        assert_eq!(state.voted_for(), None);
        assert_eq!(state.commit_index(), 0);
        assert_eq!(state.last_applied(), 0);
    }

    #[test]
    fn test_raft_state_updates() {
        let mut state = RaftState::new();
        
        state.set_node_state(NodeState::Candidate);
        assert!(matches!(state.node_state(), NodeState::Candidate));
        
        state.set_current_term(5);
        assert_eq!(state.current_term(), 5);
        
        state.set_voted_for(Some(42));
        assert_eq!(state.voted_for(), Some(42));
        
        state.set_commit_index(10);
        assert_eq!(state.commit_index(), 10);
        
        state.set_last_applied(8);
        assert_eq!(state.last_applied(), 8);
    }
}
