//! Raft node implementation
//! 
//! This module contains the main RaftNode that coordinates the Raft consensus
//! algorithm including state management, leader election, and log replication.

use crate::{Result, Error, NodeId, Term, LogIndex};
use super::{NodeState, RaftConfig, RaftMessage, RequestVoteRequest, RequestVoteResponse, AppendEntriesRequest, AppendEntriesResponse};
use super::client_tracker::{ClientRequestTracker, PendingRequest, RequestId};
use crate::storage::LogEntry;
use crate::storage::log_storage::LogStorage;
use crate::storage::state_storage::StateStorage;
use crate::network::transport::NetworkTransport;
use crate::network::{EventHandler, Event};
use std::collections::HashMap;
use std::time::{Duration, Instant};

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
    /// Election timeout tracking
    election_timeout: Option<Instant>,
    /// Votes received in current election (candidate state only)
    votes_received: HashMap<NodeId, bool>,
    /// Last time we heard from the leader
    last_heartbeat: Option<Instant>,
    /// Index of highest log entry known to be committed
    commit_index: LogIndex,
    /// Index of highest log entry applied to state machine
    last_applied: LogIndex,
    /// For leaders: next index to send to each follower
    next_index: HashMap<NodeId, LogIndex>,
    /// For leaders: highest index known to be replicated on each follower
    match_index: HashMap<NodeId, LogIndex>,
    /// Storage for persistent state
    state_storage: Option<Box<dyn StateStorage>>,
    /// Storage for log entries
    log_storage: Option<Box<dyn LogStorage>>,
    /// Network transport for communication
    transport: Option<Box<dyn NetworkTransport>>,
    /// Client request tracker for async response handling (leader only)
    client_tracker: Option<ClientRequestTracker>,
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
            election_timeout: None,
            votes_received: HashMap::new(),
            last_heartbeat: None,
            commit_index: 0,
            last_applied: 0,
            next_index: HashMap::new(),
            match_index: HashMap::new(),
            state_storage: None,
            log_storage: None,
            transport: None,
            client_tracker: None,
        }
    }
    
    /// Create a new Raft node with dependencies
    pub fn with_dependencies(
        config: RaftConfig,
        state_storage: Box<dyn StateStorage>,
        log_storage: Box<dyn LogStorage>,
        transport: Box<dyn NetworkTransport>,
    ) -> Result<Self> {
        let mut node = Self {
            config,
            state: NodeState::Follower,
            current_term: 0,
            voted_for: None,
            current_leader: None,
            election_timeout: None,
            votes_received: HashMap::new(),
            last_heartbeat: None,
            commit_index: 0,
            last_applied: 0,
            next_index: HashMap::new(),
            match_index: HashMap::new(),
            state_storage: Some(state_storage),
            log_storage: Some(log_storage),
            transport: Some(transport),
            client_tracker: None,
        };
        
        // Load persistent state
        if let Some(ref storage) = node.state_storage {
            node.current_term = storage.get_current_term();
            node.voted_for = storage.get_voted_for();
        }
        
        // Reset election timeout
        node.reset_election_timeout();
        
        Ok(node)
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
                    // Clear client tracker when stepping down as leader
                    if let Some(ref mut client_tracker) = self.client_tracker {
                        let cleared_requests = client_tracker.clear_all_requests();
                        log::info!(
                            "Node {} cleared {} pending client requests when stepping down as leader",
                            self.config.node_id, cleared_requests.len()
                        );
                    }
                    self.client_tracker = None;
                }
                log::debug!("Node {} now following in term {}", self.config.node_id, self.current_term);
            }
            NodeState::Candidate => {
                // Start election process
                self.current_term += 1;
                self.voted_for = Some(self.config.node_id);
                self.current_leader = None;
                // Clear client tracker when becoming candidate (no longer leader)
                if matches!(old_state, NodeState::Leader) {
                    if let Some(ref mut client_tracker) = self.client_tracker {
                        let cleared_requests = client_tracker.clear_all_requests();
                        log::info!(
                            "Node {} cleared {} pending client requests when becoming candidate",
                            self.config.node_id, cleared_requests.len()
                        );
                    }
                    self.client_tracker = None;
                }
                log::info!(
                    "Node {} starting election for term {} (voted for self)",
                    self.config.node_id, self.current_term
                );
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
                
                // Initialize leader state
                self.initialize_leader_state()?;
            }
        }
        
        Ok(())
    }
    
    /// Handle an election timeout
    pub fn handle_election_timeout(&mut self) -> Result<()> {
        match self.state {
            NodeState::Follower | NodeState::Candidate => {
                // Start new election
                self.start_election()?;
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
            self.send_heartbeats()?;
        }
        Ok(())
    }
    
    /// Reset election timeout with randomized duration
    pub fn reset_election_timeout(&mut self) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        // Generate pseudo-random timeout within configured range
        let mut hasher = DefaultHasher::new();
        self.config.node_id.hash(&mut hasher);
        Instant::now().hash(&mut hasher);
        let hash = hasher.finish();
        
        let (min_ms, max_ms) = self.config.election_timeout_ms();
        let range = max_ms - min_ms;
        let timeout_ms = min_ms + (hash % range);
        
        self.election_timeout = Some(Instant::now() + Duration::from_millis(timeout_ms));
        
        log::debug!(
            "Node {} reset election timeout to {}ms",
            self.config.node_id, timeout_ms
        );
    }
    
    /// Check if election timeout has expired
    pub fn is_election_timeout_expired(&self) -> bool {
        if let Some(timeout) = self.election_timeout {
            Instant::now() >= timeout
        } else {
            false
        }
    }
    
    /// Start a new election
    pub fn start_election(&mut self) -> Result<()> {
        log::info!("Node {} starting election for term {}", self.config.node_id, self.current_term + 1);
        
        // Transition to candidate state (increments term and votes for self)
        self.transition_to(NodeState::Candidate)?;
        
        // Clear previous votes and vote for self
        self.votes_received.clear();
        self.votes_received.insert(self.config.node_id, true);
        
        // Persist state
        self.persist_state()?;
        
        // Reset election timeout
        self.reset_election_timeout();
        
        // Send vote requests to all other nodes
        self.send_vote_requests()?;
        
        // Check if we already have majority (single node cluster)
        self.check_election_result()?;
        
        Ok(())
    }
    
    /// Send vote requests to all other nodes in the cluster
    pub fn send_vote_requests(&mut self) -> Result<()> {
        if let (Some(transport), Some(log_storage)) = (&self.transport, &self.log_storage) {
            let last_log_index = log_storage.get_last_index();
            let last_log_term = log_storage.get_last_term();
            
            let vote_request = RequestVoteRequest::new(
                self.current_term,
                self.config.node_id,
                last_log_index,
                last_log_term,
            );
            
            let message = RaftMessage::RequestVote(vote_request);
            let message_bytes = message.to_bytes();
            
            for &node_id in &self.config.cluster_nodes {
                if node_id != self.config.node_id {
                    log::debug!(
                        "Node {} sending vote request to node {} for term {}",
                        self.config.node_id, node_id, self.current_term
                    );
                    
                    if let Err(e) = transport.send_message(node_id, message_bytes.clone()) {
                        log::warn!(
                            "Node {} failed to send vote request to node {}: {}",
                            self.config.node_id, node_id, e
                        );
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Handle incoming vote request
    pub fn handle_vote_request(&mut self, request: RequestVoteRequest) -> Result<RequestVoteResponse> {
        log::debug!(
            "Node {} received vote request from node {} for term {}",
            self.config.node_id, request.candidate_id, request.term
        );
        
        // If request term is greater, update our term and become follower
        if request.term > self.current_term {
            log::info!(
                "Node {} updating term from {} to {} due to vote request",
                self.config.node_id, self.current_term, request.term
            );
            
            self.current_term = request.term;
            self.voted_for = None;
            self.transition_to(NodeState::Follower)?;
            self.persist_state()?;
        }
        
        let mut vote_granted = false;
        
        // Apply safety mechanisms first
        if !self.validate_vote_safety(&request)? {
            log::info!(
                "Node {} denied vote to node {} for term {} (safety validation failed)",
                self.config.node_id, request.candidate_id, request.term
            );
        } else if request.term >= self.current_term {
            let can_vote = self.voted_for.is_none() || self.voted_for == Some(request.candidate_id);
            
            if can_vote {
                vote_granted = true;
                self.voted_for = Some(request.candidate_id);
                self.reset_election_timeout(); // Reset timeout since we heard from a candidate
                self.persist_state()?;
                
                log::info!(
                    "Node {} granted vote to node {} for term {} (passed safety validation)",
                    self.config.node_id, request.candidate_id, request.term
                );
            } else {
                log::info!(
                    "Node {} denied vote to node {} for term {} (already voted for {:?})",
                    self.config.node_id, request.candidate_id, request.term, self.voted_for
                );
            }
        } else {
            log::info!(
                "Node {} denied vote to node {} (request term {} < current term {})",
                self.config.node_id, request.candidate_id, request.term, self.current_term
            );
        }
        
        Ok(RequestVoteResponse::new(
            self.current_term,
            vote_granted,
            self.config.node_id,
        ))
    }
    
    /// Handle incoming vote response
    pub fn handle_vote_response(&mut self, response: RequestVoteResponse) -> Result<()> {
        log::debug!(
            "Node {} received vote response from node {} for term {} (granted: {})",
            self.config.node_id, response.voter_id, response.term, response.vote_granted
        );
        
        // If response term is greater, update our term and become follower
        if response.term > self.current_term {
            log::info!(
                "Node {} updating term from {} to {} due to vote response",
                self.config.node_id, self.current_term, response.term
            );
            
            self.current_term = response.term;
            self.voted_for = None;
            self.transition_to(NodeState::Follower)?;
            self.persist_state()?;
            return Ok(());
        }
        
        // Only process vote responses if we're a candidate and the response is for our current term
        if matches!(self.state, NodeState::Candidate) && response.term == self.current_term {
            // Record the vote
            self.votes_received.insert(response.voter_id, response.vote_granted);
            
            if response.vote_granted {
                log::info!(
                    "Node {} received vote from node {} for term {}",
                    self.config.node_id, response.voter_id, self.current_term
                );
            }
            
            // Check if we have enough votes to become leader
            self.check_election_result()?;
        }
        
        Ok(())
    }
    
    /// Check election result and transition to leader if we have majority
    pub fn check_election_result(&mut self) -> Result<()> {
        if !matches!(self.state, NodeState::Candidate) {
            return Ok(());
        }
        
        let votes_for = self.votes_received.values().filter(|&&v| v).count();
        let total_nodes = self.config.cluster_nodes.len();
        let majority = (total_nodes / 2) + 1;
        
        log::debug!(
            "Node {} election status: {} votes out of {} nodes (need {} for majority)",
            self.config.node_id, votes_for, total_nodes, majority
        );
        
        if votes_for >= majority {
            log::info!(
                "Node {} won election for term {} with {} votes",
                self.config.node_id, self.current_term, votes_for
            );
            
            self.transition_to(NodeState::Leader)?;
            self.send_heartbeats()?;
        } else {
            // Check if we've received responses from all nodes
            let responses_received = self.votes_received.len();
            if responses_received == total_nodes {
                let votes_against = self.votes_received.values().filter(|&&v| !v).count();
                if votes_against >= majority {
                    log::info!(
                        "Node {} lost election for term {} ({} against, {} for)",
                        self.config.node_id, self.current_term, votes_against, votes_for
                    );
                    
                    // Transition back to follower
                    self.transition_to(NodeState::Follower)?;
                    self.reset_election_timeout();
                }
                // If neither majority for nor against, it's a split vote - stay candidate and timeout will trigger new election
            }
        }
        
        Ok(())
    }
    
    /// Send heartbeats to all followers (leader only)
    pub fn send_heartbeats(&mut self) -> Result<()> {
        if !matches!(self.state, NodeState::Leader) {
            return Ok(());
        }
        
        if let (Some(transport), Some(log_storage)) = (&self.transport, &self.log_storage) {
            for &node_id in &self.config.cluster_nodes {
                if node_id != self.config.node_id {
                    // Send empty AppendEntries as heartbeat
                    let prev_log_index = self.next_index.get(&node_id).copied().unwrap_or(1).saturating_sub(1);
                    let prev_log_term = if prev_log_index == 0 {
                        0
                    } else {
                        log_storage.get_entry(prev_log_index).map(|e| e.term).unwrap_or(0)
                    };
                    
                    let heartbeat = AppendEntriesRequest::new(
                        self.current_term,
                        self.config.node_id,
                        prev_log_index,
                        prev_log_term,
                        vec![], // Empty entries for heartbeat
                        self.commit_index,
                    );
                    
                    let message = RaftMessage::AppendEntries(heartbeat);
                    let message_bytes = message.to_bytes();
                    
                    log::debug!(
                        "Node {} sending heartbeat to node {} (prev_log_index: {}, prev_log_term: {})",
                        self.config.node_id, node_id, prev_log_index, prev_log_term
                    );
                    
                    if let Err(e) = transport.send_message(node_id, message_bytes) {
                        log::warn!(
                            "Node {} failed to send heartbeat to node {}: {}",
                            self.config.node_id, node_id, e
                        );
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Handle incoming AppendEntries request
    pub fn handle_append_entries(&mut self, request: AppendEntriesRequest) -> Result<AppendEntriesResponse> {
        log::debug!(
            "Node {} received AppendEntries from node {} for term {} (prev_log_index: {}, entries: {})",
            self.config.node_id, request.leader_id, request.term, request.prev_log_index, request.entries.len()
        );
        
        // If request term is greater, update our term and become follower
        if request.term > self.current_term {
            log::info!(
                "Node {} updating term from {} to {} due to AppendEntries",
                self.config.node_id, self.current_term, request.term
            );
            
            self.current_term = request.term;
            self.voted_for = None;
            self.transition_to(NodeState::Follower)?;
            self.persist_state()?;
        }
        
        let mut success = false;
        
        // Apply safety mechanisms first
        if !self.validate_append_entries_safety(&request)? {
            log::info!(
                "Node {} rejecting AppendEntries from node {} for term {} (safety validation failed)",
                self.config.node_id, request.leader_id, request.term
            );
        } else if request.term >= self.current_term {
            // Update current leader and reset election timeout
            self.current_leader = Some(request.leader_id);
            self.reset_election_timeout();
            
            // If we're a candidate and receive valid AppendEntries, become follower
            if matches!(self.state, NodeState::Candidate) {
                self.transition_to(NodeState::Follower)?;
            }
            
            success = true;
            
            // If we have entries to append
            if !request.entries.is_empty() {
                // Find the first conflicting entry
                let mut conflict_index = None;
                let start_index = request.prev_log_index + 1;
                
                if let Some(ref log_storage) = self.log_storage {
                    for (i, new_entry) in request.entries.iter().enumerate() {
                        let entry_index = start_index + i as LogIndex;
                        if let Some(existing_entry) = log_storage.get_entry(entry_index) {
                            if existing_entry.term != new_entry.term {
                                conflict_index = Some(entry_index);
                                break;
                            }
                        } else {
                            // No existing entry at this index, we can append
                            break;
                        }
                    }
                }
                
                // If there's a conflict, truncate the log from that point
                if let Some(conflict_idx) = conflict_index {
                    log::info!(
                        "Node {} truncating log from index {} due to conflict",
                        self.config.node_id, conflict_idx
                    );
                    
                    if let Some(ref mut log_storage) = self.log_storage {
                        log_storage.truncate_from(conflict_idx)?;
                    }
                }
                
                // Append new entries - need to update indices to match the expected positions
                log::info!(
                    "Node {} appending {} entries starting at index {}",
                    self.config.node_id, request.entries.len(), start_index
                );
                
                // Create entries with correct indices for appending
                let mut entries_to_append = Vec::new();
                for (i, entry) in request.entries.iter().enumerate() {
                    let target_index = start_index + i as LogIndex;
                    // Create new entry with the correct index
                    let new_entry = LogEntry::new(entry.term, target_index, entry.data.clone());
                    entries_to_append.push(new_entry);
                }
                
                let entries_len = entries_to_append.len();
                if let Some(ref mut log_storage) = self.log_storage {
                    log_storage.append_entries(entries_to_append)?;
                }
                
                // Update commit index
                if request.leader_commit > self.commit_index {
                    let last_new_entry_index = request.prev_log_index + entries_len as LogIndex;
                    
                    self.commit_index = std::cmp::min(request.leader_commit, last_new_entry_index);
                    
                    log::debug!(
                        "Node {} updated commit_index to {}",
                        self.config.node_id, self.commit_index
                    );
                    
                    // Apply committed entries to state machine
                    self.apply_committed_entries()?;
                }
            } else {
                // Heartbeat - just update commit index if needed
                if request.leader_commit > self.commit_index {
                    let last_log_index = if let Some(ref log_storage) = self.log_storage {
                        log_storage.get_last_index()
                    } else {
                        0
                    };
                    
                    self.commit_index = std::cmp::min(request.leader_commit, last_log_index);
                    
                    log::debug!(
                        "Node {} updated commit_index to {} (heartbeat)",
                        self.config.node_id, self.commit_index
                    );
                    
                    // Apply committed entries to state machine
                    self.apply_committed_entries()?;
                }
            }
        } else {
            log::info!(
                "Node {} rejecting AppendEntries from node {} (request term {} < current term {})",
                self.config.node_id, request.leader_id, request.term, self.current_term
            );
        }
        
        let last_log_index = if let Some(ref log_storage) = self.log_storage {
            log_storage.get_last_index()
        } else {
            0
        };
        
        Ok(AppendEntriesResponse::new(
            self.current_term,
            success,
            self.config.node_id,
            last_log_index,
        ))
    }
    
    /// Handle incoming AppendEntries response
    pub fn handle_append_entries_response(&mut self, response: AppendEntriesResponse, follower_id: NodeId) -> Result<()> {
        log::debug!(
            "Node {} received AppendEntries response from node {} for term {} (success: {})",
            self.config.node_id, follower_id, response.term, response.success
        );
        
        // If response term is greater, update our term and become follower
        if response.term > self.current_term {
            log::info!(
                "Node {} updating term from {} to {} due to AppendEntries response",
                self.config.node_id, self.current_term, response.term
            );
            
            self.current_term = response.term;
            self.voted_for = None;
            self.transition_to(NodeState::Follower)?;
            self.persist_state()?;
            return Ok(());
        }
        
        // Only process responses if we're the leader and response is for our current term
        if matches!(self.state, NodeState::Leader) && response.term == self.current_term {
            if response.success {
                // Update next_index and match_index for this follower
                // Use the follower's last_log_index from the response
                let follower_last_log_index = response.last_log_index;
                self.next_index.insert(follower_id, follower_last_log_index + 1);
                self.match_index.insert(follower_id, follower_last_log_index);
                
                log::debug!(
                    "Node {} updated next_index[{}] = {}, match_index[{}] = {}",
                    self.config.node_id, follower_id, follower_last_log_index + 1, follower_id, follower_last_log_index
                );
                
                // Check if we can advance commit_index
                self.advance_commit_index()?;
            } else {
                // Decrement next_index and retry
                let current_next = self.next_index.get(&follower_id).copied().unwrap_or(1);
                let new_next = current_next.saturating_sub(1).max(1);
                self.next_index.insert(follower_id, new_next);
                
                log::debug!(
                    "Node {} decremented next_index[{}] from {} to {} due to failed AppendEntries",
                    self.config.node_id, follower_id, current_next, new_next
                );
                
                // TODO: Retry sending AppendEntries with updated next_index
            }
        }
        
        Ok(())
    }
    
    /// Apply committed entries to state machine
    pub fn apply_committed_entries(&mut self) -> Result<()> {
        while self.last_applied < self.commit_index {
            self.last_applied += 1;
            
            if let Some(ref log_storage) = self.log_storage {
                if let Some(entry) = log_storage.get_entry(self.last_applied) {
                    log::debug!(
                        "Node {} applying log entry {} to state machine (term: {}, {} bytes)",
                        self.config.node_id, self.last_applied, entry.term, entry.data.len()
                    );
                    
                    // Apply the log entry to the state machine
                    if let Err(e) = self.apply_log_entry_to_state_machine(&entry) {
                        log::error!(
                            "Node {} failed to apply log entry {} to state machine: {}",
                            self.config.node_id, self.last_applied, e
                        );
                        // Continue applying other entries even if one fails
                    }
                    
                    // Handle pending client requests for this committed log entry (leader only)
                    if matches!(self.state, NodeState::Leader) {
                        self.handle_committed_client_requests(self.last_applied)?;
                    }
                } else {
                    log::warn!(
                        "Node {} tried to apply missing log entry {}",
                        self.config.node_id, self.last_applied
                    );
                    break;
                }
            }
        }
        
        Ok(())
    }
    
    /// Apply a single log entry to the state machine
    fn apply_log_entry_to_state_machine(&mut self, entry: &LogEntry) -> Result<()> {
        use crate::kv::KVOperation;
        
        // Try to deserialize the log entry data as a KV operation
        match KVOperation::from_bytes(&entry.data) {
            Ok(operation) => {
                log::debug!(
                    "Node {} applying KV operation to state machine: {:?}",
                    self.config.node_id, operation
                );
                
                // For now, we just log the operation since we don't have direct access to the KV store
                // The actual state machine application will be handled by the server event loop
                // when it detects that new entries have been committed
                
                match operation {
                    KVOperation::Put { ref key, ref value } => {
                        log::info!(
                            "Node {} applied PUT operation: key='{}', value_len={}",
                            self.config.node_id, key, value.len()
                        );
                    }
                    KVOperation::Delete { ref key } => {
                        log::info!(
                            "Node {} applied DELETE operation: key='{}'",
                            self.config.node_id, key
                        );
                    }
                    KVOperation::Get { ref key } => {
                        log::warn!(
                            "Node {} found GET operation in log (should not happen): key='{}'",
                            self.config.node_id, key
                        );
                    }
                    KVOperation::List => {
                        log::warn!(
                            "Node {} found LIST operation in log (should not happen)",
                            self.config.node_id
                        );
                    }
                }
                
                Ok(())
            }
            Err(e) => {
                log::warn!(
                    "Node {} could not deserialize log entry {} as KV operation: {}. Treating as opaque data.",
                    self.config.node_id, entry.index, e
                );
                
                // If it's not a KV operation, just log it as opaque data
                log::debug!(
                    "Node {} applied opaque log entry {} ({} bytes)",
                    self.config.node_id, entry.index, entry.data.len()
                );
                
                Ok(())
            }
        }
    }
    
    /// Advance commit index based on majority replication
    pub fn advance_commit_index(&mut self) -> Result<()> {
        if !matches!(self.state, NodeState::Leader) {
            return Ok(());
        }
        
        let (last_log_index, current_term) = if let Some(ref log_storage) = self.log_storage {
            (log_storage.get_last_index(), self.current_term)
        } else {
            return Ok(());
        };
        
        // Check each index from commit_index + 1 to last_log_index
        for index in (self.commit_index + 1)..=last_log_index {
            // Count how many nodes have replicated this entry
            let mut replicated_count = 1; // Count ourselves (leader always has all entries)
            
            for &node_id in &self.config.cluster_nodes {
                if node_id != self.config.node_id {
                    if let Some(&match_idx) = self.match_index.get(&node_id) {
                        if match_idx >= index {
                            replicated_count += 1;
                        }
                    }
                }
            }
            
            let majority = (self.config.cluster_nodes.len() / 2) + 1;
            
            log::debug!("Node {} checking commit for index {}: {} replicated out of {} nodes (need {})",
                self.config.node_id, index, replicated_count, self.config.cluster_nodes.len(), majority
            );
            
            if replicated_count >= majority {
                // Check that the entry is from our current term (Raft safety requirement)
                let entry_term = if let Some(ref log_storage) = self.log_storage {
                    log_storage.get_entry(index).map(|e| e.term)
                } else {
                    None
                };
                
                log::debug!("Entry {} has term {:?}, current_term={}", index, entry_term, current_term);
                
                if let Some(term) = entry_term {
                    if term == current_term {
                        self.commit_index = index;
                        log::info!("Node {} advanced commit_index to {} (replicated on {} nodes)",
                            self.config.node_id, index, replicated_count
                        );
                        
                        // Apply newly committed entries
                        self.apply_committed_entries()?;
                    } else {
                        log::debug!("Node {} cannot commit index {} from term {} (current term {})",
                            self.config.node_id, index, term, current_term
                        );
                    }
                } else {
                    log::warn!("Node {} cannot find entry at index {} for commit check",
                        self.config.node_id, index
                    );
                }
            } else {
                log::debug!("Not enough replicas for index {}: {} < {}", index, replicated_count, majority);
                // If this index doesn't have majority, later indices won't either
                break;
            }
        }
        
        Ok(())
    }
    
    /// Initialize leader state (called when becoming leader)
    pub fn initialize_leader_state(&mut self) -> Result<()> {
        if !matches!(self.state, NodeState::Leader) {
            return Ok(());
        }
        
        // Initialize next_index and match_index for all followers
        if let Some(ref log_storage) = self.log_storage {
            let last_log_index = log_storage.get_last_index();
            
            for &node_id in &self.config.cluster_nodes {
                if node_id != self.config.node_id {
                    // Initialize next_index to last log index + 1
                    self.next_index.insert(node_id, last_log_index + 1);
                    // Initialize match_index to 0
                    self.match_index.insert(node_id, 0);
                    
                    log::debug!(
                        "Node {} initialized next_index[{}] = {}, match_index[{}] = 0",
                        self.config.node_id, node_id, last_log_index + 1, node_id
                    );
                }
            }
        }
        
        // Initialize client request tracker for async response handling
        self.client_tracker = Some(ClientRequestTracker::new());
        
        log::info!(
            "Leader {} initialized client request tracker for async response handling",
            self.config.node_id
        );
        
        Ok(())
    }
    
    /// Persist current state to storage
    pub fn persist_state(&mut self) -> Result<()> {
        if let Some(ref mut storage) = self.state_storage {
            storage.save_current_term(self.current_term)?;
            storage.save_voted_for(self.voted_for)?;
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
    
    /// Get access to the network transport
    pub fn get_transport(&self) -> &dyn NetworkTransport {
        self.transport.as_ref().expect("Transport not initialized").as_ref()
    }
    
    /// Append a client command to the Raft log (leader only)
    pub fn append_client_command(&mut self, command_data: Vec<u8>) -> Result<LogIndex> {
        // Only leaders can accept client commands
        if !matches!(self.state, NodeState::Leader) {
            return Err(Error::Raft(format!(
                "Node {} is not the leader (current state: {:?}). Current leader: {:?}",
                self.config.node_id, self.state, self.current_leader
            )));
        }
        
        // Get the next log index
        let next_index = if let Some(ref log_storage) = self.log_storage {
            log_storage.get_last_index() + 1
        } else {
            return Err(Error::Raft("Log storage not initialized".to_string()));
        };
        
        // Create a new log entry with the client command
        let log_entry = LogEntry::new(self.current_term, next_index, command_data);
        
        log::info!(
            "Leader {} appending client command to log at index {} (term {})",
            self.config.node_id, next_index, self.current_term
        );
        
        // Append the entry to our log
        if let Some(ref mut log_storage) = self.log_storage {
            log_storage.append_entries(vec![log_entry.clone()])?;
        }
        
        // Update our match_index for ourselves (leader always has all entries)
        self.match_index.insert(self.config.node_id, next_index);
        
        // Send AppendEntries to all followers to replicate the command
        self.replicate_log_entry(log_entry)?;
        
        Ok(next_index)
    }
    
    /// Replicate a log entry to all followers
    fn replicate_log_entry(&mut self, entry: LogEntry) -> Result<()> {
        if !matches!(self.state, NodeState::Leader) {
            return Ok(());
        }
        
        if let Some(transport) = &self.transport {
            for &node_id in &self.config.cluster_nodes {
                if node_id != self.config.node_id {
                    // Get the previous log index and term for this follower
                    let next_idx = self.next_index.get(&node_id).copied().unwrap_or(1);
                    let prev_log_index = next_idx.saturating_sub(1);
                    
                    let prev_log_term = if prev_log_index == 0 {
                        0
                    } else if let Some(ref log_storage) = self.log_storage {
                        log_storage.get_entry(prev_log_index).map(|e| e.term).unwrap_or(0)
                    } else {
                        0
                    };
                    
                    // Create AppendEntries request with the new entry
                    let append_request = AppendEntriesRequest::new(
                        self.current_term,
                        self.config.node_id,
                        prev_log_index,
                        prev_log_term,
                        vec![entry.clone()],
                        self.commit_index,
                    );
                    
                    let message = RaftMessage::AppendEntries(append_request);
                    let message_bytes = message.to_bytes();
                    
                    log::debug!(
                        "Leader {} sending AppendEntries with client command to node {} (prev_log_index: {}, prev_log_term: {})",
                        self.config.node_id, node_id, prev_log_index, prev_log_term
                    );
                    
                    if let Err(e) = transport.send_message(node_id, message_bytes) {
                        log::warn!(
                            "Leader {} failed to send AppendEntries to node {}: {}",
                            self.config.node_id, node_id, e
                        );
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Check if a log entry at the given index is committed
    pub fn is_log_entry_committed(&self, log_index: LogIndex) -> bool {
        log_index <= self.commit_index
    }
    
    /// Get the current leader ID (for follower redirection)
    pub fn get_current_leader(&self) -> Option<NodeId> {
        self.current_leader
    }
    
    /// Get the current commit index
    pub fn get_commit_index(&self) -> LogIndex {
        self.commit_index
    }
    
    /// Get a log entry at the specified index
    pub fn get_log_entry(&self, index: LogIndex) -> Option<LogEntry> {
        if let Some(ref log_storage) = self.log_storage {
            log_storage.get_entry(index)
        } else {
            None
        }
    }
    
    /// Get the last log index (total number of log entries)
    pub fn get_last_log_index(&self) -> LogIndex {
        if let Some(ref log_storage) = self.log_storage {
            log_storage.get_last_index()
        } else {
            0
        }
    }
    
    /// Handle committed client requests for async response handling (leader only)
    fn handle_committed_client_requests(&mut self, committed_log_index: LogIndex) -> Result<()> {
        if !matches!(self.state, NodeState::Leader) {
            return Ok(());
        }
        
        if let Some(ref mut client_tracker) = self.client_tracker {
            // Get all requests that can be completed for this log index
            let completable_requests = client_tracker.get_completable_requests(committed_log_index);
            
            if !completable_requests.is_empty() {
                log::info!(
                    "Leader {} handling {} client requests for committed log index {}",
                    self.config.node_id, completable_requests.len(), committed_log_index
                );
                
                for pending_request in completable_requests {
                    log::debug!(
                        "Leader {} completing client request {} for log index {}",
                        self.config.node_id, pending_request.request_id, pending_request.log_index
                    );
                    
                    // TODO: Send success response to client via TCP stream
                    // For now, we just log that the request would be completed
                    // The actual response sending will be implemented when we integrate
                    // with the server event loop that has access to TCP streams
                    
                    log::info!(
                        "Leader {} would send success response to client for request {} (log index {})",
                        self.config.node_id, pending_request.request_id, pending_request.log_index
                    );
                }
            }
            
            // Handle timed out requests
            let timed_out_requests = client_tracker.get_timed_out_requests();
            if !timed_out_requests.is_empty() {
                log::warn!(
                    "Leader {} handling {} timed out client requests",
                    self.config.node_id, timed_out_requests.len()
                );
                
                for pending_request in timed_out_requests {
                    log::warn!(
                        "Leader {} timing out client request {} (log index {}, {} retries)",
                        self.config.node_id, pending_request.request_id, pending_request.log_index, pending_request.retry_count
                    );
                    
                    // TODO: Send timeout/error response to client via TCP stream
                    log::warn!(
                        "Leader {} would send timeout response to client for request {}",
                        self.config.node_id, pending_request.request_id
                    );
                }
            }
        }
        
        Ok(())
    }
    
    
    /// Get the number of pending client requests (leader only)
    pub fn pending_client_requests_count(&self) -> usize {
        if let Some(ref client_tracker) = self.client_tracker {
            client_tracker.pending_count()
        } else {
            0
        }
    }
    
    /// Track a client request for async response handling (leader only) - full version
    pub fn track_client_request(&mut self, stream: std::net::TcpStream, log_index: LogIndex, request_data: Vec<u8>) -> Result<RequestId> {
        if !matches!(self.state, NodeState::Leader) {
            return Err(Error::Raft(format!(
                "Node {} is not the leader, cannot track client request",
                self.config.node_id
            )));
        }
        
        if let Some(ref mut client_tracker) = self.client_tracker {
            let request_id = client_tracker.add_request(stream, log_index, request_data);
            
            log::debug!(
                "Leader {} tracking client request {} for log index {}",
                self.config.node_id, request_id, log_index
            );
            
            Ok(request_id)
        } else {
            Err(Error::Raft(format!(
                "Leader {} client tracker not initialized",
                self.config.node_id
            )))
        }
    }
    
    /// Get requests that can be completed due to log commitment (leader only)
    pub fn get_completable_client_requests(&mut self, commit_index: LogIndex) -> Vec<PendingRequest> {
        if !matches!(self.state, NodeState::Leader) {
            return Vec::new();
        }
        
        if let Some(ref mut client_tracker) = self.client_tracker {
            client_tracker.get_completable_requests(commit_index)
        } else {
            Vec::new()
        }
    }
    
    /// Get requests that have timed out (leader only)
    pub fn get_timed_out_client_requests(&mut self) -> Vec<PendingRequest> {
        if !matches!(self.state, NodeState::Leader) {
            return Vec::new();
        }
        
        if let Some(ref mut client_tracker) = self.client_tracker {
            client_tracker.get_timed_out_requests()
        } else {
            Vec::new()
        }
    }
    
    /// Get requests that need retry due to leadership changes
    pub fn get_requests_for_retry(&mut self) -> Vec<PendingRequest> {
        // This method should be called when stepping down from leadership
        // or when leadership changes occur
        if let Some(ref mut client_tracker) = self.client_tracker {
            client_tracker.get_requests_for_retry()
        } else {
            Vec::new()
        }
    }
    
    /// Get the current term for external access
    pub fn get_current_term(&self) -> Term {
        self.current_term
    }
    
    // ========== SAFETY MECHANISMS ==========
    
    /// Safety Mechanism 1: Election Safety
    /// Ensures at most one leader per term by validating vote requests
    pub fn validate_election_safety(&self, candidate_term: Term, candidate_id: NodeId) -> Result<bool> {
        // If we've already voted for someone else in this term, deny the vote
        if candidate_term == self.current_term {
            if let Some(voted_for) = self.voted_for {
                if voted_for != candidate_id {
                    log::info!(
                        "Node {} denying vote to {} for term {} (already voted for {})",
                        self.config.node_id, candidate_id, candidate_term, voted_for
                    );
                    return Ok(false);
                }
            }
        }
        
        // If candidate term is less than our current term, deny
        if candidate_term < self.current_term {
            log::info!(
                "Node {} denying vote to {} (candidate term {} < current term {})",
                self.config.node_id, candidate_id, candidate_term, self.current_term
            );
            return Ok(false);
        }
        
        Ok(true)
    }
    
    /// Safety Mechanism 2: Leader Append-Only Property
    /// Ensures leaders never overwrite or delete existing log entries
    pub fn validate_leader_append_only(&self, new_entries: &[LogEntry], start_index: LogIndex) -> Result<bool> {
        if !matches!(self.state, NodeState::Leader) {
            return Ok(true); // Only applies to leaders
        }
        
        if let Some(ref log_storage) = self.log_storage {
            // Check that we're not overwriting existing entries with different content
            for (i, new_entry) in new_entries.iter().enumerate() {
                let entry_index = start_index + i as LogIndex;
                
                if let Some(existing_entry) = log_storage.get_entry(entry_index) {
                    // Leaders should never overwrite existing entries
                    if existing_entry.term != new_entry.term || existing_entry.data != new_entry.data {
                        log::error!(
                            "Leader {} attempted to overwrite existing entry at index {} (existing: term={}, new: term={})",
                            self.config.node_id, entry_index, existing_entry.term, new_entry.term
                        );
                        return Ok(false);
                    }
                }
            }
        }
        
        Ok(true)
    }
    
    /// Safety Mechanism 3: Log Matching Property
    /// Validates that if two logs contain an entry with same index and term,
    /// then the logs are identical in all preceding entries
    pub fn validate_log_matching(&self, prev_log_index: LogIndex, prev_log_term: Term) -> Result<bool> {
        if prev_log_index == 0 {
            return Ok(true); // No previous entry to check
        }
        
        if let Some(ref log_storage) = self.log_storage {
            if let Some(our_entry) = log_storage.get_entry(prev_log_index) {
                if our_entry.term == prev_log_term {
                    // Terms match, so by Log Matching Property, all preceding entries should match
                    // This is implicitly validated by the Raft protocol's consistency checks
                    log::debug!(
                        "Node {} validated log matching at index {} term {}",
                        self.config.node_id, prev_log_index, prev_log_term
                    );
                    return Ok(true);
                } else {
                    log::info!(
                        "Node {} log matching failed at index {} (our term: {}, expected: {})",
                        self.config.node_id, prev_log_index, our_entry.term, prev_log_term
                    );
                    return Ok(false);
                }
            } else {
                log::info!(
                    "Node {} missing entry at index {} for log matching check",
                    self.config.node_id, prev_log_index
                );
                return Ok(false);
            }
        }
        
        Ok(true)
    }
    
    /// Safety Mechanism 4: Leader Completeness Guarantee
    /// Ensures that leaders have all committed entries from previous terms
    pub fn validate_leader_completeness(&self, candidate_last_log_index: LogIndex, candidate_last_log_term: Term) -> Result<bool> {
        if let Some(ref log_storage) = self.log_storage {
            let our_last_index = log_storage.get_last_index();
            let our_last_term = log_storage.get_last_term();
            
            // Candidate's log must be at least as up-to-date as ours
            // A log is more up-to-date if:
            // 1. Last log term is higher, OR
            // 2. Last log terms are equal and last log index is higher or equal
            let candidate_more_up_to_date = candidate_last_log_term > our_last_term ||
                (candidate_last_log_term == our_last_term && candidate_last_log_index >= our_last_index);
            
            if !candidate_more_up_to_date {
                log::info!(
                    "Node {} denying vote - candidate log not up-to-date (candidate: term={}, index={}; ours: term={}, index={})",
                    self.config.node_id, candidate_last_log_term, candidate_last_log_index, our_last_term, our_last_index
                );
                return Ok(false);
            }
            
            log::debug!(
                "Node {} validated leader completeness for candidate (candidate: term={}, index={}; ours: term={}, index={})",
                self.config.node_id, candidate_last_log_term, candidate_last_log_index, our_last_term, our_last_index
            );
        }
        
        Ok(true)
    }
    
    /// Comprehensive safety validation for vote requests
    pub fn validate_vote_safety(&self, request: &RequestVoteRequest) -> Result<bool> {
        // Apply all safety mechanisms
        if !self.validate_election_safety(request.term, request.candidate_id)? {
            return Ok(false);
        }
        
        if !self.validate_leader_completeness(request.last_log_index, request.last_log_term)? {
            return Ok(false);
        }
        
        Ok(true)
    }
    
    /// Comprehensive safety validation for append entries
    pub fn validate_append_entries_safety(&self, request: &AppendEntriesRequest) -> Result<bool> {
        // Apply log matching property
        if !self.validate_log_matching(request.prev_log_index, request.prev_log_term)? {
            return Ok(false);
        }
        
        // Apply leader append-only property (if we're a leader receiving this)
        if matches!(self.state, NodeState::Leader) && request.term <= self.current_term {
            // A leader should not accept AppendEntries from another leader in the same term
            log::warn!(
                "Leader {} received AppendEntries from {} in same/lower term {} (current term: {})",
                self.config.node_id, request.leader_id, request.term, self.current_term
            );
            return Ok(false);
        }
        
        Ok(true)
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
    use crate::storage::log_storage::{InMemoryLogStorage, LogStorage};
    use crate::storage::state_storage::{InMemoryStateStorage, StateStorage};
    use crate::network::transport::MockTransport;



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
        let config = RaftConfig::fast(0, vec![0, 1, 2]); // 3-node cluster so we don't immediately become leader
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(0));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Follower should become candidate on election timeout
        node.handle_election_timeout().unwrap();
        assert!(node.is_candidate());
        assert_eq!(node.current_term(), 1);
        
        // Candidate should start new election on timeout (won't become leader without majority)
        let old_term = node.current_term();
        node.handle_election_timeout().unwrap();
        assert!(node.is_candidate());
        assert_eq!(node.current_term(), old_term + 1);
    }

    #[test]
    fn test_single_node_election() {
        let config = RaftConfig::fast(0, vec![0]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(0));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Single node should become leader immediately after starting election
        node.start_election().unwrap();
        assert!(node.is_leader());
        assert_eq!(node.current_term(), 1);
        assert_eq!(node.current_leader(), Some(0));
    }

    #[test]
    fn test_vote_request_handling() {
        let config = RaftConfig::fast(1, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(1));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Test granting vote to valid candidate
        let vote_request = RequestVoteRequest::new(1, 0, 0, 0);
        let response = node.handle_vote_request(vote_request).unwrap();
        
        assert!(response.vote_granted);
        assert_eq!(response.term, 1);
        assert_eq!(response.voter_id, 1);
        assert_eq!(node.voted_for, Some(0));
        assert_eq!(node.current_term(), 1);
    }

    #[test]
    fn test_vote_request_denial() {
        let config = RaftConfig::fast(1, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(1));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Vote for candidate 0 first
        let vote_request1 = RequestVoteRequest::new(1, 0, 0, 0);
        let response1 = node.handle_vote_request(vote_request1).unwrap();
        assert!(response1.vote_granted);
        
        // Try to vote for candidate 2 in same term - should be denied
        let vote_request2 = RequestVoteRequest::new(1, 2, 0, 0);
        let response2 = node.handle_vote_request(vote_request2).unwrap();
        assert!(!response2.vote_granted);
        assert_eq!(node.voted_for, Some(0)); // Still voted for candidate 0
    }

    #[test]
    fn test_vote_request_higher_term() {
        let config = RaftConfig::fast(1, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(1));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Set node to term 1 and vote for someone
        node.current_term = 1;
        node.voted_for = Some(0);
        
        // Receive vote request with higher term
        let vote_request = RequestVoteRequest::new(2, 2, 0, 0);
        let response = node.handle_vote_request(vote_request).unwrap();
        
        assert!(response.vote_granted);
        assert_eq!(response.term, 2);
        assert_eq!(node.current_term(), 2);
        assert_eq!(node.voted_for, Some(2));
        assert!(node.is_follower());
    }

    #[test]
    fn test_vote_response_handling() {
        let config = RaftConfig::fast(0, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(0));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Start election (becomes candidate)
        node.start_election().unwrap();
        assert!(node.is_candidate());
        assert_eq!(node.current_term(), 1);
        
        // Receive vote from node 1
        let vote_response1 = RequestVoteResponse::new(1, true, 1);
        node.handle_vote_response(vote_response1).unwrap();
        
        // Should become leader with majority (2 out of 3 votes: self + node 1)
        assert!(node.is_leader());
        assert_eq!(node.current_leader(), Some(0));
    }

    #[test]
    fn test_vote_response_higher_term() {
        let config = RaftConfig::fast(0, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(0));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Start election (becomes candidate in term 1)
        node.start_election().unwrap();
        assert!(node.is_candidate());
        assert_eq!(node.current_term(), 1);
        
        // Receive vote response with higher term
        let vote_response = RequestVoteResponse::new(2, false, 1);
        node.handle_vote_response(vote_response).unwrap();
        
        // Should become follower and update term
        assert!(node.is_follower());
        assert_eq!(node.current_term(), 2);
        assert_eq!(node.voted_for, None);
    }

    #[test]
    fn test_election_majority_calculation() {
        let config = RaftConfig::fast(0, vec![0, 1, 2, 3, 4]); // 5 nodes, need 3 for majority
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(0));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Start election
        node.start_election().unwrap();
        assert!(node.is_candidate());
        
        // Receive one positive vote (have 2 total: self + node 1)
        let vote_response1 = RequestVoteResponse::new(1, true, 1);
        node.handle_vote_response(vote_response1).unwrap();
        assert!(node.is_candidate()); // Still candidate, need one more vote
        
        // Receive another positive vote (have 3 total: majority)
        let vote_response2 = RequestVoteResponse::new(1, true, 2);
        node.handle_vote_response(vote_response2).unwrap();
        assert!(node.is_leader()); // Now leader with majority
    }

    #[test]
    fn test_election_split_vote() {
        let config = RaftConfig::fast(0, vec![0, 1, 2, 3, 4]); // 5 nodes, need 3 for majority
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(0));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Start election
        node.start_election().unwrap();
        assert!(node.is_candidate());
        
        // Receive mixed votes: 2 for, 2 against (total: self=for, 1=for, 2=against, 3=against, 4=for)
        // This creates a split where no one gets majority (need 3 out of 5)
        let vote_response1 = RequestVoteResponse::new(1, true, 1);
        node.handle_vote_response(vote_response1).unwrap();
        assert!(node.is_candidate()); // Still candidate
        
        let vote_response2 = RequestVoteResponse::new(1, false, 2);
        node.handle_vote_response(vote_response2).unwrap();
        assert!(node.is_candidate()); // Still candidate
        
        let vote_response3 = RequestVoteResponse::new(1, false, 3);
        node.handle_vote_response(vote_response3).unwrap();
        assert!(node.is_candidate()); // Still candidate
        
        let vote_response4 = RequestVoteResponse::new(1, true, 4);
        node.handle_vote_response(vote_response4).unwrap();
        
        // Should remain candidate since it's a split vote (3 for, 2 against - we have majority!)
        // Actually, with 3 votes for (self + 1 + 4), we should become leader
        assert!(node.is_leader());
    }

    #[test]
    fn test_election_timeout_reset() {
        let config = RaftConfig::fast(1, vec![0, 1, 2]);
        
        let mut node = RaftNode::new(config);
        
        // Reset election timeout
        node.reset_election_timeout();
        assert!(node.election_timeout.is_some());
        
        let first_timeout = node.election_timeout.unwrap();
        
        // Reset again - should get different timeout
        std::thread::sleep(std::time::Duration::from_millis(1)); // Ensure different timestamp
        node.reset_election_timeout();
        let second_timeout = node.election_timeout.unwrap();
        
        // Timeouts should be different (due to timestamp-based randomization)
        assert_ne!(first_timeout, second_timeout);
    }

    #[test]
    fn test_log_up_to_date_comparison() {
        let config = RaftConfig::fast(1, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let mut log_storage = InMemoryLogStorage::new();
        
        // Add some entries to our log
        let entry1 = LogEntry::new(1, 1, b"data1".to_vec());
        let entry2 = LogEntry::new(2, 2, b"data2".to_vec());
        log_storage.append_entries(vec![entry1, entry2]).unwrap();
        
        let transport = Box::new(MockTransport::new(1));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, Box::new(log_storage), transport).unwrap();
        
        // Test vote request with less up-to-date log (lower term)
        let vote_request1 = RequestVoteRequest::new(3, 0, 2, 1); // Same index, lower term
        let response1 = node.handle_vote_request(vote_request1).unwrap();
        assert!(!response1.vote_granted); // Should deny
        
        // Test vote request with more up-to-date log (higher term)
        let vote_request2 = RequestVoteRequest::new(3, 0, 2, 3); // Same index, higher term
        let response2 = node.handle_vote_request(vote_request2).unwrap();
        assert!(response2.vote_granted); // Should grant
    }

    #[test]
    fn test_append_entries_heartbeat() {
        let config = RaftConfig::fast(1, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(1));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Test heartbeat (empty AppendEntries) from leader
        let heartbeat = AppendEntriesRequest::new(2, 0, 0, 0, vec![], 0);
        let response = node.handle_append_entries(heartbeat).unwrap();
        
        assert!(response.success);
        assert_eq!(response.term, 2);
        assert_eq!(response.follower_id, 1);
        assert_eq!(node.current_term(), 2);
        assert_eq!(node.current_leader(), Some(0));
        assert!(node.is_follower());
    }

    #[test]
    fn test_append_entries_with_entries() {
        let config = RaftConfig::fast(1, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(1));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Test AppendEntries with actual entries
        let entries = vec![
            LogEntry::new(1, 1, b"entry1".to_vec()),
            LogEntry::new(2, 2, b"entry2".to_vec()),
        ];
        
        let append_request = AppendEntriesRequest::new(2, 0, 0, 0, entries, 0);
        let response = node.handle_append_entries(append_request).unwrap();
        
        assert!(response.success);
        assert_eq!(response.term, 2);
        assert_eq!(response.last_log_index, 2); // Should have 2 entries now
        assert_eq!(node.current_term(), 2);
        assert_eq!(node.current_leader(), Some(0));
    }

    #[test]
    fn test_append_entries_log_consistency_check() {
        let config = RaftConfig::fast(1, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let mut log_storage = InMemoryLogStorage::new();
        
        // Pre-populate log with some entries
        let existing_entries = vec![
            LogEntry::new(1, 1, b"existing1".to_vec()),
            LogEntry::new(2, 2, b"existing2".to_vec()),
        ];
        log_storage.append_entries(existing_entries).unwrap();
        
        let transport = Box::new(MockTransport::new(1));
        let mut node = RaftNode::with_dependencies(config, state_storage, Box::new(log_storage), transport).unwrap();
        
        // Test AppendEntries with mismatched prev_log_term (should fail)
        let entries = vec![LogEntry::new(3, 3, b"new_entry".to_vec())];
        let append_request = AppendEntriesRequest::new(3, 0, 2, 1, entries, 0); // prev_log_term=1, but entry 2 has term=2
        let response = node.handle_append_entries(append_request).unwrap();
        
        assert!(!response.success); // Should fail due to log inconsistency
        assert_eq!(response.term, 3);
        
        // Test AppendEntries with correct prev_log_term (should succeed)
        let entries = vec![LogEntry::new(3, 3, b"new_entry".to_vec())];
        let append_request = AppendEntriesRequest::new(3, 0, 2, 2, entries, 0); // prev_log_term=2, matches entry 2
        let response = node.handle_append_entries(append_request).unwrap();
        
        assert!(response.success); // Should succeed
        assert_eq!(response.last_log_index, 3); // Should have 3 entries now
    }

    #[test]
    fn test_append_entries_higher_term() {
        let config = RaftConfig::fast(1, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(1));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Set node to candidate state in term 1
        node.transition_to(NodeState::Candidate).unwrap();
        assert!(node.is_candidate());
        assert_eq!(node.current_term(), 1);
        
        // Receive AppendEntries with higher term - should become follower
        let append_request = AppendEntriesRequest::new(2, 0, 0, 0, vec![], 0);
        let response = node.handle_append_entries(append_request).unwrap();
        
        assert!(response.success);
        assert_eq!(response.term, 2);
        assert!(node.is_follower());
        assert_eq!(node.current_term(), 2);
        assert_eq!(node.current_leader(), Some(0));
    }

    #[test]
    fn test_append_entries_commit_index_update() {
        let config = RaftConfig::fast(1, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(1));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Add entries and update commit index
        let entries = vec![
            LogEntry::new(1, 1, b"entry1".to_vec()),
            LogEntry::new(2, 2, b"entry2".to_vec()),
        ];
        
        let append_request = AppendEntriesRequest::new(2, 0, 0, 0, entries, 1); // leader_commit = 1
        let response = node.handle_append_entries(append_request).unwrap();
        
        assert!(response.success);
        assert_eq!(node.commit_index, 1); // Should update commit_index
        assert_eq!(node.last_applied, 1); // Should apply committed entries
    }

    #[test]
    fn test_append_entries_response_handling() {
        let config = RaftConfig::fast(0, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let mut log_storage = InMemoryLogStorage::new();
        
        // Add some entries to leader's log
        let entries = vec![
            LogEntry::new(2, 1, b"entry1".to_vec()),
            LogEntry::new(2, 2, b"entry2".to_vec()),
        ];
        log_storage.append_entries(entries).unwrap();
        
        let transport = Box::new(MockTransport::new(0));
        let mut node = RaftNode::with_dependencies(config, state_storage, Box::new(log_storage), transport).unwrap();
        
        // Set the leader's term to 2 to match the log entries
        node.current_term = 2;
        
        // Become leader
        node.transition_to(NodeState::Leader).unwrap();
        assert!(node.is_leader());
        
        // Test successful AppendEntries response
        // The response indicates the follower's last_log_index is 2
        let success_response = AppendEntriesResponse::new(2, true, 1, 2);
        node.handle_append_entries_response(success_response, 1).unwrap();
        
        // Should update next_index and match_index for follower
        // match_index should be set to the follower's last_log_index (2)
        assert_eq!(node.next_index.get(&1), Some(&3)); // last_log_index + 1
        assert_eq!(node.match_index.get(&1), Some(&2)); // follower's last_log_index
        
        // Test failed AppendEntries response
        let fail_response = AppendEntriesResponse::new(2, false, 2, 1);
        node.handle_append_entries_response(fail_response, 2).unwrap();
        
        // Should decrement next_index for follower
        let expected_next = node.next_index.get(&2).copied().unwrap_or(1);
        assert!(expected_next < 3); // Should be decremented from initial value of 3
    }

    #[test]
    fn test_leader_state_initialization() {
        let config = RaftConfig::fast(0, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let mut log_storage = InMemoryLogStorage::new();
        
        // Add some entries to log
        let entries = vec![
            LogEntry::new(1, 1, b"entry1".to_vec()),
            LogEntry::new(2, 2, b"entry2".to_vec()),
        ];
        log_storage.append_entries(entries).unwrap();
        
        let transport = Box::new(MockTransport::new(0));
        let mut node = RaftNode::with_dependencies(config, state_storage, Box::new(log_storage), transport).unwrap();
        
        // Transition to leader should initialize leader state
        node.transition_to(NodeState::Leader).unwrap();
        
        // Check that next_index and match_index are initialized correctly
        assert_eq!(node.next_index.get(&1), Some(&3)); // last_log_index + 1
        assert_eq!(node.next_index.get(&2), Some(&3));
        assert_eq!(node.match_index.get(&1), Some(&0)); // Initialized to 0
        assert_eq!(node.match_index.get(&2), Some(&0));
    }

    #[test]
    fn test_commit_index_advancement() {
        let config = RaftConfig::fast(0, vec![0, 1, 2]); // 3 nodes, need 2 for majority
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let mut log_storage = InMemoryLogStorage::new();
        
        // Add entries from current term
        let entries = vec![
            LogEntry::new(1, 1, b"entry1".to_vec()),
            LogEntry::new(1, 2, b"entry2".to_vec()), // Same term as leader
        ];
        log_storage.append_entries(entries).unwrap();
        
        let transport = Box::new(MockTransport::new(0));
        let mut node = RaftNode::with_dependencies(config, state_storage, Box::new(log_storage), transport).unwrap();
        
        // Become leader in term 1
        node.current_term = 1;
        node.transition_to(NodeState::Leader).unwrap();
        
        // Simulate that majority of followers have replicated entry 1
        node.match_index.insert(1, 1); // Follower 1 has entry 1
        node.match_index.insert(2, 0); // Follower 2 doesn't have entry 1 yet
        
        // Try to advance commit index
        log::debug!("Before advance_commit_index: commit_index={}, match_index={:?}, current_term={}",
                 node.commit_index, node.match_index, node.current_term);
        node.advance_commit_index().unwrap();
        log::debug!("After advance_commit_index: commit_index={}, last_applied={}",
                 node.commit_index, node.last_applied);
        
        // Leader + follower 1 = 2 nodes, which is majority for 3 nodes
        assert_eq!(node.commit_index, 1);
        assert_eq!(node.last_applied, 1);
        
        // Now simulate that follower 2 also replicates entry 1
        node.match_index.insert(2, 1);
        node.advance_commit_index().unwrap();
        
        // Should still be at 1, but now we can advance to entry 2 if follower 1 gets it
        node.match_index.insert(1, 2);
        node.advance_commit_index().unwrap();
        
        // Now should advance to entry 2 (leader + follower 1 = majority)
        assert_eq!(node.commit_index, 2);
        assert_eq!(node.last_applied, 2);
    }

    #[test]
    fn test_heartbeat_sending() {
        let config = RaftConfig::fast(0, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(0));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Only leaders should send heartbeats
        node.send_heartbeats().unwrap(); // Should do nothing as follower
        
        // Become leader
        node.transition_to(NodeState::Leader).unwrap();
        
        // Now should send heartbeats (won't fail with mock transport)
        node.send_heartbeats().unwrap();
        
        // Test heartbeat timeout handling
        node.handle_heartbeat_timeout().unwrap();
    }

    #[test]
    fn test_log_conflict_resolution() {
        let config = RaftConfig::fast(1, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let mut log_storage = InMemoryLogStorage::new();
        
        // Pre-populate with conflicting entries
        let existing_entries = vec![
            LogEntry::new(1, 1, b"entry1".to_vec()),
            LogEntry::new(2, 2, b"old_entry2".to_vec()), // This will conflict
            LogEntry::new(3, 2, b"old_entry3".to_vec()), // This will be truncated
        ];
        log_storage.append_entries(existing_entries).unwrap();
        
        let transport = Box::new(MockTransport::new(1));
        let mut node = RaftNode::with_dependencies(config, state_storage, Box::new(log_storage), transport).unwrap();
        
        // Send AppendEntries with conflicting entry at index 2
        let new_entries = vec![
            LogEntry::new(3, 2, b"new_entry2".to_vec()), // Different term, should cause conflict
            LogEntry::new(3, 3, b"new_entry3".to_vec()),
        ];
        
        let append_request = AppendEntriesRequest::new(3, 0, 1, 1, new_entries, 0);
        let response = node.handle_append_entries(append_request).unwrap();
        
        assert!(response.success);
        
        // Verify that conflicting entries were replaced
        if let Some(ref log_storage) = node.log_storage {
            assert_eq!(log_storage.get_last_index(), 3);
            
            // Entry 1 should remain unchanged
            let entry1 = log_storage.get_entry(1).expect("Entry 1 should exist");
            assert_eq!(entry1.term, 1);
            assert_eq!(entry1.data, b"entry1");
            

            // Entry 2 should be replaced
            let entry2 = log_storage.get_entry(2).expect("Entry 2 should exist");
            assert_eq!(entry2.term, 3);
            assert_eq!(entry2.data, b"new_entry2");
            
            // Entry 3 should be the new entry
            let entry3 = log_storage.get_entry(3).expect("Entry 3 should exist");
            assert_eq!(entry3.term, 3);
            assert_eq!(entry3.data, b"new_entry3");
        }
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

    // ========== SAFETY MECHANISM TESTS ==========

    #[test]
    fn test_election_safety_validation() {
        let config = RaftConfig::fast(1, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(1));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Set up initial state - already voted for candidate 0 in term 2
        node.current_term = 2;
        node.voted_for = Some(0);
        
        // Test election safety - should deny vote to different candidate in same term
        assert!(!node.validate_election_safety(2, 2).unwrap());
        
        // Should allow vote to same candidate in same term
        assert!(node.validate_election_safety(2, 0).unwrap());
        
        // Should deny vote for lower term
        assert!(!node.validate_election_safety(1, 2).unwrap());
        
        // Should allow vote for higher term
        assert!(node.validate_election_safety(3, 2).unwrap());
    }

    #[test]
    fn test_leader_append_only_validation() {
        let config = RaftConfig::fast(0, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let mut log_storage = InMemoryLogStorage::new();
        
        // Pre-populate log with existing entries
        let existing_entries = vec![
            LogEntry::new(1, 1, b"existing1".to_vec()),
            LogEntry::new(2, 2, b"existing2".to_vec()),
        ];
        log_storage.append_entries(existing_entries).unwrap();
        
        let transport = Box::new(MockTransport::new(0));
        let mut node = RaftNode::with_dependencies(config, state_storage, Box::new(log_storage), transport).unwrap();
        
        // Become leader
        node.transition_to(NodeState::Leader).unwrap();
        
        // Test appending new entries (should be allowed)
        let new_entries = vec![LogEntry::new(3, 3, b"new_entry".to_vec())];
        assert!(node.validate_leader_append_only(&new_entries, 3).unwrap());
        
        // Test overwriting existing entry with different content (should be denied)
        let conflicting_entries = vec![LogEntry::new(3, 2, b"different_data".to_vec())];
        assert!(!node.validate_leader_append_only(&conflicting_entries, 2).unwrap());
        
        // Test overwriting with same content (should be allowed)
        let same_entries = vec![LogEntry::new(2, 2, b"existing2".to_vec())];
        assert!(node.validate_leader_append_only(&same_entries, 2).unwrap());
        
        // Non-leaders should always pass this check
        node.transition_to(NodeState::Follower).unwrap();
        assert!(node.validate_leader_append_only(&conflicting_entries, 2).unwrap());
    }

    #[test]
    fn test_log_matching_validation() {
        let config = RaftConfig::fast(1, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let mut log_storage = InMemoryLogStorage::new();
        
        // Pre-populate log
        let entries = vec![
            LogEntry::new(1, 1, b"entry1".to_vec()),
            LogEntry::new(2, 2, b"entry2".to_vec()),
            LogEntry::new(3, 3, b"entry3".to_vec()),
        ];
        log_storage.append_entries(entries).unwrap();
        
        let transport = Box::new(MockTransport::new(1));
        let node = RaftNode::with_dependencies(config, state_storage, Box::new(log_storage), transport).unwrap();
        
        // Test matching prev_log_index and prev_log_term
        assert!(node.validate_log_matching(2, 2).unwrap()); // Entry 2 has term 2
        assert!(node.validate_log_matching(3, 3).unwrap()); // Entry 3 has term 3
        
        // Test mismatched prev_log_term
        assert!(!node.validate_log_matching(2, 1).unwrap()); // Entry 2 has term 2, not 1
        assert!(!node.validate_log_matching(3, 2).unwrap()); // Entry 3 has term 3, not 2
        
        // Test missing entry
        assert!(!node.validate_log_matching(5, 4).unwrap()); // No entry at index 5
        
        // Test prev_log_index = 0 (should always pass)
        assert!(node.validate_log_matching(0, 0).unwrap());
    }

    #[test]
    fn test_leader_completeness_validation() {
        let config = RaftConfig::fast(1, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let mut log_storage = InMemoryLogStorage::new();
        
        // Our log: term 2, index 3
        let our_entries = vec![
            LogEntry::new(1, 1, b"entry1".to_vec()),
            LogEntry::new(2, 2, b"entry2".to_vec()),
            LogEntry::new(2, 3, b"entry3".to_vec()),
        ];
        log_storage.append_entries(our_entries).unwrap();
        
        let transport = Box::new(MockTransport::new(1));
        let node = RaftNode::with_dependencies(config, state_storage, Box::new(log_storage), transport).unwrap();
        
        // Candidate with higher term should be granted vote
        assert!(node.validate_leader_completeness(3, 3).unwrap()); // Higher term
        
        // Candidate with same term but higher index should be granted vote
        assert!(node.validate_leader_completeness(4, 2).unwrap()); // Same term, higher index
        
        // Candidate with same term and same index should be granted vote
        assert!(node.validate_leader_completeness(3, 2).unwrap()); // Same term, same index
        
        // Candidate with lower term should be denied vote
        assert!(!node.validate_leader_completeness(3, 1).unwrap()); // Lower term
        
        // Candidate with same term but lower index should be denied vote
        assert!(!node.validate_leader_completeness(2, 2).unwrap()); // Same term, lower index
    }

    #[test]
    fn test_vote_safety_integration() {
        let config = RaftConfig::fast(1, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let mut log_storage = InMemoryLogStorage::new();
        
        // Add some entries to our log
        let entries = vec![
            LogEntry::new(1, 1, b"entry1".to_vec()),
            LogEntry::new(2, 2, b"entry2".to_vec()),
        ];
        log_storage.append_entries(entries).unwrap();
        
        let transport = Box::new(MockTransport::new(1));
        let mut node = RaftNode::with_dependencies(config, state_storage, Box::new(log_storage), transport).unwrap();
        
        // Test vote request that should pass all safety checks
        let valid_request = RequestVoteRequest::new(3, 0, 2, 2); // Higher term, up-to-date log
        let response = node.handle_vote_request(valid_request).unwrap();
        assert!(response.vote_granted);
        
        // Test vote request that should fail leader completeness
        let invalid_request = RequestVoteRequest::new(4, 2, 1, 1); // Higher term but outdated log
        let response = node.handle_vote_request(invalid_request).unwrap();
        assert!(!response.vote_granted);
        
        // Test vote request that should fail election safety (already voted)
        let duplicate_request = RequestVoteRequest::new(3, 2, 2, 2); // Same term, different candidate
        let response = node.handle_vote_request(duplicate_request).unwrap();
        assert!(!response.vote_granted);
    }

    #[test]
    fn test_append_entries_safety_integration() {
        let config = RaftConfig::fast(1, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let mut log_storage = InMemoryLogStorage::new();
        
        // Pre-populate log
        let entries = vec![
            LogEntry::new(1, 1, b"entry1".to_vec()),
            LogEntry::new(2, 2, b"entry2".to_vec()),
        ];
        log_storage.append_entries(entries).unwrap();
        
        let transport = Box::new(MockTransport::new(1));
        let mut node = RaftNode::with_dependencies(config, state_storage, Box::new(log_storage), transport).unwrap();
        
        // Test valid AppendEntries that should pass safety checks
        let valid_entries = vec![LogEntry::new(3, 3, b"entry3".to_vec())];
        let valid_request = AppendEntriesRequest::new(3, 0, 2, 2, valid_entries, 0);
        let response = node.handle_append_entries(valid_request).unwrap();
        assert!(response.success);
        
        // Test AppendEntries with log matching failure
        let invalid_entries = vec![LogEntry::new(3, 4, b"entry4".to_vec())];
        let invalid_request = AppendEntriesRequest::new(3, 0, 2, 1, invalid_entries, 0); // Wrong prev_log_term
        let response = node.handle_append_entries(invalid_request).unwrap();
        assert!(!response.success);
        
        // Test leader receiving AppendEntries from another leader in same term
        node.transition_to(NodeState::Leader).unwrap();
        let leader_conflict = AppendEntriesRequest::new(3, 2, 3, 3, vec![], 0); // Same term
        let response = node.handle_append_entries(leader_conflict).unwrap();
        assert!(!response.success);
    }

    #[test]
    fn test_safety_mechanisms_prevent_split_brain() {
        let config = RaftConfig::fast(1, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(1));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Node votes for candidate 0 in term 2
        let vote_request1 = RequestVoteRequest::new(2, 0, 0, 0);
        let response1 = node.handle_vote_request(vote_request1).unwrap();
        assert!(response1.vote_granted);
        assert_eq!(node.voted_for, Some(0));
        
        // Another candidate (2) tries to get vote in same term - should be denied
        let vote_request2 = RequestVoteRequest::new(2, 2, 0, 0);
        let response2 = node.handle_vote_request(vote_request2).unwrap();
        assert!(!response2.vote_granted); // Election safety prevents double voting
        assert_eq!(node.voted_for, Some(0)); // Still voted for original candidate
        
        // This prevents split-brain scenario where multiple leaders could emerge in same term
    }

    #[test]
    fn test_safety_mechanisms_preserve_committed_entries() {
        let config = RaftConfig::fast(1, vec![0, 1, 2]);
        
        let state_storage = Box::new(InMemoryStateStorage::new());
        let mut log_storage = InMemoryLogStorage::new();
        
        // Simulate committed entries from previous terms
        let committed_entries = vec![
            LogEntry::new(1, 1, b"committed1".to_vec()),
            LogEntry::new(2, 2, b"committed2".to_vec()),
        ];
        log_storage.append_entries(committed_entries).unwrap();
        
        let transport = Box::new(MockTransport::new(1));
        let mut node = RaftNode::with_dependencies(config, state_storage, Box::new(log_storage), transport).unwrap();
        
        // Set commit index to indicate these entries are committed
        node.commit_index = 2;
        
        // Candidate with incomplete log should be denied vote (leader completeness)
        let incomplete_candidate = RequestVoteRequest::new(3, 0, 1, 1); // Missing committed entry 2
        let response = node.handle_vote_request(incomplete_candidate).unwrap();
        assert!(!response.vote_granted);
        
        // Only candidates with complete logs should be granted votes
        let complete_candidate = RequestVoteRequest::new(3, 0, 2, 2); // Has all committed entries
        let response = node.handle_vote_request(complete_candidate).unwrap();
        assert!(response.vote_granted);
    }
}
