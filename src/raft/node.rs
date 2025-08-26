//! Raft node implementation
//! 
//! This module contains the main RaftNode that coordinates the Raft consensus
//! algorithm including state management, leader election, and log replication.

use crate::{Result, Error, NodeId, Term, LogIndex};
use super::{NodeState, RaftConfig};
use crate::storage::LogEntry;
use crate::storage::log_storage::LogStorage;
use crate::storage::state_storage::StateStorage;
use crate::network::transport::NetworkTransport;
use crate::network::{EventHandler, Event};

/// Main Raft node implementation
pub struct RaftNode {
    /// Configuration for this node
    config: RaftConfig,
    /// Current state of the node
    state: NodeState,
    /// Current term
    current_term: Term,
    /// Node voted for in current term
    voted_for: Option<NodeId>,
    /// Current leader (if known)
    current_leader: Option<NodeId>,
}

impl RaftNode {
    /// Create a new Raft node
    pub fn new(config: RaftConfig) -> Self {
        Self {
            config,
            state: NodeState::Follower,
            current_term: 0,
            voted_for: None,
            current_leader: None,
        }
    }
    
    /// Get the current state of the node
    pub fn state(&self) -> &NodeState {
        &self.state
    }
    
    /// Get the current term
    pub fn current_term(&self) -> Term {
        self.current_term
    }
    
    /// Get the node ID
    pub fn node_id(&self) -> NodeId {
        self.config.node_id
    }
    
    /// Get the current leader (if known)
    pub fn current_leader(&self) -> Option<NodeId> {
        self.current_leader
    }
    
    /// Check if this node is the leader
    pub fn is_leader(&self) -> bool {
        matches!(self.state, NodeState::Leader)
    }
    
    /// Check if this node is a follower
    pub fn is_follower(&self) -> bool {
        matches!(self.state, NodeState::Follower)
    }
    
    /// Check if this node is a candidate
    pub fn is_candidate(&self) -> bool {
        matches!(self.state, NodeState::Candidate)
    }
    
    /// Transition to a new state
    pub fn transition_to(&mut self, new_state: NodeState) -> Result<()> {
        let old_state = self.state.clone();
        
        log::info!(
            "Node {} transitioning from {:?} to {:?} in term {}",
            self.config.node_id, old_state, new_state, self.current_term
        );
        
        self.state = new_state;
        
        // Perform state-specific initialization
        match &self.state {
            NodeState::Follower => {
                // Reset election timer, clear leader if transitioning from leader
                if matches!(old_state, NodeState::Leader) {
                    log::info!("Node {} stepping down as leader", self.config.node_id);
                    self.current_leader = None;
                }
                log::debug!("Node {} now following in term {}", self.config.node_id, self.current_term);
            }
            NodeState::Candidate => {
                // Start election process
                self.current_term += 1;
                self.voted_for = Some(self.config.node_id);
                self.current_leader = None;
                log::info!(
                    "Node {} starting election for term {} (voted for self)",
                    self.config.node_id, self.current_term
                );
            }
            NodeState::Leader => {
                // Become leader
                self.current_leader = Some(self.config.node_id);
                self.voted_for = None;
                log::info!(
                    "Node {} became leader for term {}",
                    self.config.node_id, self.current_term
                );
            }
        }
        
        Ok(())
    }
    
    /// Handle an election timeout
    pub fn handle_election_timeout(&mut self) -> Result<()> {
        match self.state {
            NodeState::Follower | NodeState::Candidate => {
                // Start new election
                self.transition_to(NodeState::Candidate)?;
            }
            NodeState::Leader => {
                // Leaders don't have election timeouts
            }
        }
        Ok(())
    }
    
    /// Handle a heartbeat timeout (for leaders)
    pub fn handle_heartbeat_timeout(&mut self) -> Result<()> {
        if matches!(self.state, NodeState::Leader) {
            // Send heartbeats to all followers
            // This would be implemented with actual network calls
        }
        Ok(())
    }
    
    /// Get configuration
    pub fn config(&self) -> &RaftConfig {
        &self.config
    }
    
    /// Update configuration
    pub fn update_config(&mut self, config: RaftConfig) {
        self.config = config;
    }
}

impl EventHandler for RaftNode {
    fn handle_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Raft(raft_event) => {
                use crate::network::RaftEvent;
                match raft_event {
                    RaftEvent::ElectionTimeout => self.handle_election_timeout(),
                    RaftEvent::HeartbeatTimeout => self.handle_heartbeat_timeout(),
                    RaftEvent::StateTransition { from: _, to: _ } => {
                        // State transition already handled
                        Ok(())
                    }
                    RaftEvent::LogCommitted { index: _ } => {
                        // Log commit handling would be implemented here
                        Ok(())
                    }
                }
            }
            Event::Network(_) => {
                // Network event handling would be implemented here
                Ok(())
            }
            Event::Client(_) => {
                // Client request handling would be implemented here
                Ok(())
            }
            Event::Timer(_) => {
                // Timer event handling would be implemented here
                Ok(())
            }
        }
    }
}

/// Mock Raft node for testing
#[derive(Debug)]
pub struct MockRaftNode {
    /// Node ID
    node_id: NodeId,
    /// Current state
    state: NodeState,
    /// Events received
    events_received: Vec<Event>,
    /// Whether to return errors
    should_error: bool,
}

impl MockRaftNode {
    /// Create a new mock Raft node
    pub fn new(node_id: NodeId) -> Self {
        Self {
            node_id,
            state: NodeState::Follower,
            events_received: Vec::new(),
            should_error: false,
        }
    }
    
    /// Set whether the mock should return errors
    pub fn set_should_error(&mut self, should_error: bool) {
        self.should_error = should_error;
    }
    
    /// Get events received
    pub fn events_received(&self) -> &[Event] {
        &self.events_received
    }
    
    /// Clear received events
    pub fn clear_events(&mut self) {
        self.events_received.clear();
    }
    
    /// Get current state
    pub fn state(&self) -> &NodeState {
        &self.state
    }
    
    /// Set state
    pub fn set_state(&mut self, state: NodeState) {
        self.state = state;
    }
}

impl EventHandler for MockRaftNode {
    fn handle_event(&mut self, event: Event) -> Result<()> {
        self.events_received.push(event);
        
        if self.should_error {
            Err(Error::Raft("Mock error".to_string()))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raft_node_creation() {
        let config = RaftConfig::default();
        let node = RaftNode::new(config);
        
        assert_eq!(node.node_id(), 0);
        assert!(node.is_follower());
        assert_eq!(node.current_term(), 0);
        assert_eq!(node.current_leader(), None);
    }

    #[test]
    fn test_state_transitions() {
        let config = RaftConfig::default();
        let mut node = RaftNode::new(config);
        
        // Test transition to candidate
        node.transition_to(NodeState::Candidate).unwrap();
        assert!(node.is_candidate());
        assert_eq!(node.current_term(), 1);
        assert_eq!(node.voted_for, Some(0));
        
        // Test transition to leader
        node.transition_to(NodeState::Leader).unwrap();
        assert!(node.is_leader());
        assert_eq!(node.current_leader(), Some(0));
        
        // Test transition back to follower
        node.transition_to(NodeState::Follower).unwrap();
        assert!(node.is_follower());
        assert_eq!(node.current_leader(), None);
    }

    #[test]
    fn test_election_timeout_handling() {
        let config = RaftConfig::default();
        let mut node = RaftNode::new(config);
        
        // Follower should become candidate on election timeout
        node.handle_election_timeout().unwrap();
        assert!(node.is_candidate());
        assert_eq!(node.current_term(), 1);
        
        // Candidate should start new election on timeout
        let old_term = node.current_term();
        node.handle_election_timeout().unwrap();
        assert!(node.is_candidate());
        assert_eq!(node.current_term(), old_term + 1);
    }

    #[test]
    fn test_mock_raft_node() {
        let mut mock_node = MockRaftNode::new(1);
        
        assert_eq!(mock_node.node_id, 1);
        assert!(matches!(mock_node.state(), NodeState::Follower));
        
        // Test event handling
        let event = Event::Raft(crate::network::RaftEvent::ElectionTimeout);
        mock_node.handle_event(event.clone()).unwrap();
        
        let events = mock_node.events_received();
        assert_eq!(events.len(), 1);
        
        // Test error handling
        mock_node.set_should_error(true);
        let result = mock_node.handle_event(event);
        assert!(result.is_err());
    }
}
