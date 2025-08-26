//! Raft message types
//! 
//! This module defines the message types used in the Raft consensus protocol
//! for communication between nodes.

use crate::{NodeId, Term, LogIndex};
use crate::storage::LogEntry;

/// Main Raft message type
#[derive(Debug, Clone, PartialEq)]
pub enum RaftMessage {
    /// Request vote message for leader election
    RequestVote(RequestVoteRequest),
    /// Response to request vote
    RequestVoteResponse(RequestVoteResponse),
    /// Append entries message for log replication and heartbeats
    AppendEntries(AppendEntriesRequest),
    /// Response to append entries
    AppendEntriesResponse(AppendEntriesResponse),
}

/// Request vote message for leader election
#[derive(Debug, Clone, PartialEq)]
pub struct RequestVoteRequest {
    /// Candidate's term
    pub term: Term,
    /// Candidate requesting vote
    pub candidate_id: NodeId,
    /// Index of candidate's last log entry
    pub last_log_index: LogIndex,
    /// Term of candidate's last log entry
    pub last_log_term: Term,
}

impl RequestVoteRequest {
    /// Create a new request vote message
    pub fn new(term: Term, candidate_id: NodeId, last_log_index: LogIndex, last_log_term: Term) -> Self {
        Self {
            term,
            candidate_id,
            last_log_index,
            last_log_term,
        }
    }
}

/// Response to request vote message
#[derive(Debug, Clone, PartialEq)]
pub struct RequestVoteResponse {
    /// Current term, for candidate to update itself
    pub term: Term,
    /// True means candidate received vote
    pub vote_granted: bool,
    /// Node that sent this response
    pub voter_id: NodeId,
}

impl RequestVoteResponse {
    /// Create a new request vote response
    pub fn new(term: Term, vote_granted: bool, voter_id: NodeId) -> Self {
        Self {
            term,
            vote_granted,
            voter_id,
        }
    }
}

/// Append entries message for log replication and heartbeats
#[derive(Debug, Clone, PartialEq)]
pub struct AppendEntriesRequest {
    /// Leader's term
    pub term: Term,
    /// Leader's ID
    pub leader_id: NodeId,
    /// Index of log entry immediately preceding new ones
    pub prev_log_index: LogIndex,
    /// Term of prev_log_index entry
    pub prev_log_term: Term,
    /// Log entries to store (empty for heartbeat)
    pub entries: Vec<LogEntry>,
    /// Leader's commit index
    pub leader_commit: LogIndex,
}

impl AppendEntriesRequest {
    /// Create a new append entries message
    pub fn new(
        term: Term,
        leader_id: NodeId,
        prev_log_index: LogIndex,
        prev_log_term: Term,
        entries: Vec<LogEntry>,
        leader_commit: LogIndex,
    ) -> Self {
        Self {
            term,
            leader_id,
            prev_log_index,
            prev_log_term,
            entries,
            leader_commit,
        }
    }
    
    /// Create a heartbeat message (empty entries)
    pub fn heartbeat(term: Term, leader_id: NodeId, prev_log_index: LogIndex, prev_log_term: Term, leader_commit: LogIndex) -> Self {
        Self::new(term, leader_id, prev_log_index, prev_log_term, Vec::new(), leader_commit)
    }
    
    /// Check if this is a heartbeat message
    pub fn is_heartbeat(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Response to append entries message
#[derive(Debug, Clone, PartialEq)]
pub struct AppendEntriesResponse {
    /// Current term, for leader to update itself
    pub term: Term,
    /// True if follower contained entry matching prev_log_index and prev_log_term
    pub success: bool,
    /// Node that sent this response
    pub follower_id: NodeId,
    /// For optimization: the index of the last log entry
    pub last_log_index: LogIndex,
}

impl AppendEntriesResponse {
    /// Create a new append entries response
    pub fn new(term: Term, success: bool, follower_id: NodeId, last_log_index: LogIndex) -> Self {
        Self {
            term,
            success,
            follower_id,
            last_log_index,
        }
    }
}

impl RaftMessage {
    /// Get the term from any Raft message
    pub fn term(&self) -> Term {
        match self {
            RaftMessage::RequestVote(req) => req.term,
            RaftMessage::RequestVoteResponse(resp) => resp.term,
            RaftMessage::AppendEntries(req) => req.term,
            RaftMessage::AppendEntriesResponse(resp) => resp.term,
        }
    }
    
    /// Get the sender ID from any Raft message (if applicable)
    pub fn sender_id(&self) -> Option<NodeId> {
        match self {
            RaftMessage::RequestVote(req) => Some(req.candidate_id),
            RaftMessage::RequestVoteResponse(resp) => Some(resp.voter_id),
            RaftMessage::AppendEntries(req) => Some(req.leader_id),
            RaftMessage::AppendEntriesResponse(resp) => Some(resp.follower_id),
        }
    }
    
    /// Serialize message to bytes (simple implementation)
    pub fn to_bytes(&self) -> Vec<u8> {
        // This is a very simple serialization - in a real implementation
        // you would use a proper serialization format like protobuf or bincode
        match self {
            RaftMessage::RequestVote(req) => {
                let mut bytes = vec![0u8]; // Message type
                bytes.extend_from_slice(&req.term.to_be_bytes());
                bytes.extend_from_slice(&req.candidate_id.to_be_bytes());
                bytes.extend_from_slice(&req.last_log_index.to_be_bytes());
                bytes.extend_from_slice(&req.last_log_term.to_be_bytes());
                bytes
            }
            RaftMessage::RequestVoteResponse(resp) => {
                let mut bytes = vec![1u8]; // Message type
                bytes.extend_from_slice(&resp.term.to_be_bytes());
                bytes.push(if resp.vote_granted { 1 } else { 0 });
                bytes.extend_from_slice(&resp.voter_id.to_be_bytes());
                bytes
            }
            RaftMessage::AppendEntries(req) => {
                let mut bytes = vec![2u8]; // Message type
                bytes.extend_from_slice(&req.term.to_be_bytes());
                bytes.extend_from_slice(&req.leader_id.to_be_bytes());
                bytes.extend_from_slice(&req.prev_log_index.to_be_bytes());
                bytes.extend_from_slice(&req.prev_log_term.to_be_bytes());
                bytes.extend_from_slice(&req.leader_commit.to_be_bytes());
                bytes.extend_from_slice(&(req.entries.len() as u32).to_be_bytes());
                // For simplicity, we'll skip serializing the actual entries
                bytes
            }
            RaftMessage::AppendEntriesResponse(resp) => {
                let mut bytes = vec![3u8]; // Message type
                bytes.extend_from_slice(&resp.term.to_be_bytes());
                bytes.push(if resp.success { 1 } else { 0 });
                bytes.extend_from_slice(&resp.follower_id.to_be_bytes());
                bytes.extend_from_slice(&resp.last_log_index.to_be_bytes());
                bytes
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_vote_message() {
        let req = RequestVoteRequest::new(5, 1, 10, 4);
        
        assert_eq!(req.term, 5);
        assert_eq!(req.candidate_id, 1);
        assert_eq!(req.last_log_index, 10);
        assert_eq!(req.last_log_term, 4);
        
        let msg = RaftMessage::RequestVote(req.clone());
        assert_eq!(msg.term(), 5);
        assert_eq!(msg.sender_id(), Some(1));
    }

    #[test]
    fn test_request_vote_response() {
        let resp = RequestVoteResponse::new(5, true, 2);
        
        assert_eq!(resp.term, 5);
        assert!(resp.vote_granted);
        assert_eq!(resp.voter_id, 2);
        
        let msg = RaftMessage::RequestVoteResponse(resp);
        assert_eq!(msg.term(), 5);
        assert_eq!(msg.sender_id(), Some(2));
    }

    #[test]
    fn test_append_entries_request() {
        let entries = vec![
            LogEntry::new(3, 5, b"data1".to_vec()),
            LogEntry::new(3, 6, b"data2".to_vec()),
        ];
        
        let req = AppendEntriesRequest::new(3, 1, 4, 2, entries, 4);
        
        assert_eq!(req.term, 3);
        assert_eq!(req.leader_id, 1);
        assert_eq!(req.prev_log_index, 4);
        assert_eq!(req.prev_log_term, 2);
        assert_eq!(req.entries.len(), 2);
        assert_eq!(req.leader_commit, 4);
        assert!(!req.is_heartbeat());
        
        // Test heartbeat
        let heartbeat = AppendEntriesRequest::heartbeat(3, 1, 4, 2, 4);
        assert!(heartbeat.is_heartbeat());
        assert_eq!(heartbeat.entries.len(), 0);
    }

    #[test]
    fn test_append_entries_response() {
        let resp = AppendEntriesResponse::new(3, true, 2, 6);
        
        assert_eq!(resp.term, 3);
        assert!(resp.success);
        assert_eq!(resp.follower_id, 2);
        assert_eq!(resp.last_log_index, 6);
        
        let msg = RaftMessage::AppendEntriesResponse(resp);
        assert_eq!(msg.term(), 3);
        assert_eq!(msg.sender_id(), Some(2));
    }

    #[test]
    fn test_message_serialization() {
        let req = RequestVoteRequest::new(5, 1, 10, 4);
        let msg = RaftMessage::RequestVote(req);
        
        let bytes = msg.to_bytes();
        assert!(!bytes.is_empty());
        assert_eq!(bytes[0], 0); // Message type for RequestVote
    }
}
