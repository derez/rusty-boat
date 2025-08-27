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
    
    /// Serialize message to bytes with complete implementation
    pub fn to_bytes(&self) -> Vec<u8> {
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
                
                // Serialize each log entry
                for entry in &req.entries {
                    bytes.extend_from_slice(&entry.term.to_be_bytes());
                    bytes.extend_from_slice(&entry.index.to_be_bytes());
                    bytes.extend_from_slice(&(entry.data.len() as u32).to_be_bytes());
                    bytes.extend_from_slice(&entry.data);
                }
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
    
    /// Deserialize message from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, crate::Error> {
        if bytes.is_empty() {
            return Err(crate::Error::Network("Empty message".to_string()));
        }
        
        let msg_type = bytes[0];
        let mut offset = 1;
        
        match msg_type {
            0 => {
                // RequestVote
                if bytes.len() < 33 { // 1 + 8 + 8 + 8 + 8
                    return Err(crate::Error::Network("RequestVote message too short".to_string()));
                }
                
                let term = u64::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                    bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                ]);
                offset += 8;
                
                let candidate_id = u64::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                    bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                ]);
                offset += 8;
                
                let last_log_index = u64::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                    bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                ]);
                offset += 8;
                
                let last_log_term = u64::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                    bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                ]);
                
                Ok(RaftMessage::RequestVote(RequestVoteRequest::new(
                    term, candidate_id, last_log_index, last_log_term
                )))
            }
            1 => {
                // RequestVoteResponse
                if bytes.len() < 18 { // 1 + 8 + 1 + 8
                    return Err(crate::Error::Network("RequestVoteResponse message too short".to_string()));
                }
                
                let term = u64::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                    bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                ]);
                offset += 8;
                
                let vote_granted = bytes[offset] != 0;
                offset += 1;
                
                let voter_id = u64::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                    bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                ]);
                
                Ok(RaftMessage::RequestVoteResponse(RequestVoteResponse::new(
                    term, vote_granted, voter_id
                )))
            }
            2 => {
                // AppendEntries
                if bytes.len() < 45 { // 1 + 8 + 8 + 8 + 8 + 8 + 4
                    return Err(crate::Error::Network("AppendEntries message too short".to_string()));
                }
                
                let term = u64::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                    bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                ]);
                offset += 8;
                
                let leader_id = u64::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                    bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                ]);
                offset += 8;
                
                let prev_log_index = u64::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                    bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                ]);
                offset += 8;
                
                let prev_log_term = u64::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                    bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                ]);
                offset += 8;
                
                let leader_commit = u64::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                    bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                ]);
                offset += 8;
                
                let entries_count = u32::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                ]);
                offset += 4;
                
                let mut entries = Vec::new();
                for _ in 0..entries_count {
                    if offset + 20 > bytes.len() { // 8 + 8 + 4 minimum for entry header
                        return Err(crate::Error::Network("AppendEntries entry header too short".to_string()));
                    }
                    
                    let entry_term = u64::from_be_bytes([
                        bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                        bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                    ]);
                    offset += 8;
                    
                    let entry_index = u64::from_be_bytes([
                        bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                        bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                    ]);
                    offset += 8;
                    
                    let data_len = u32::from_be_bytes([
                        bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                    ]) as usize;
                    offset += 4;
                    
                    if offset + data_len > bytes.len() {
                        return Err(crate::Error::Network("AppendEntries entry data too short".to_string()));
                    }
                    
                    let data = bytes[offset..offset + data_len].to_vec();
                    offset += data_len;
                    
                    entries.push(crate::storage::LogEntry::new(entry_term, entry_index, data));
                }
                
                Ok(RaftMessage::AppendEntries(AppendEntriesRequest::new(
                    term, leader_id, prev_log_index, prev_log_term, entries, leader_commit
                )))
            }
            3 => {
                // AppendEntriesResponse
                if bytes.len() < 26 { // 1 + 8 + 1 + 8 + 8
                    return Err(crate::Error::Network("AppendEntriesResponse message too short".to_string()));
                }
                
                let term = u64::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                    bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                ]);
                offset += 8;
                
                let success = bytes[offset] != 0;
                offset += 1;
                
                let follower_id = u64::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                    bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                ]);
                offset += 8;
                
                let last_log_index = u64::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                    bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                ]);
                
                Ok(RaftMessage::AppendEntriesResponse(AppendEntriesResponse::new(
                    term, success, follower_id, last_log_index
                )))
            }
            _ => Err(crate::Error::Network(format!("Unknown message type: {}", msg_type))),
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

    #[test]
    fn test_request_vote_serialization_roundtrip() {
        let original = RaftMessage::RequestVote(RequestVoteRequest::new(5, 1, 10, 4));
        let bytes = original.to_bytes();
        let deserialized = RaftMessage::from_bytes(&bytes).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_request_vote_response_serialization_roundtrip() {
        let original = RaftMessage::RequestVoteResponse(RequestVoteResponse::new(5, true, 2));
        let bytes = original.to_bytes();
        let deserialized = RaftMessage::from_bytes(&bytes).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_append_entries_serialization_roundtrip() {
        let entries = vec![
            LogEntry::new(3, 5, b"data1".to_vec()),
            LogEntry::new(3, 6, b"data2".to_vec()),
        ];
        let original = RaftMessage::AppendEntries(AppendEntriesRequest::new(3, 1, 4, 2, entries, 4));
        let bytes = original.to_bytes();
        let deserialized = RaftMessage::from_bytes(&bytes).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_append_entries_response_serialization_roundtrip() {
        let original = RaftMessage::AppendEntriesResponse(AppendEntriesResponse::new(3, true, 2, 6));
        let bytes = original.to_bytes();
        let deserialized = RaftMessage::from_bytes(&bytes).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_heartbeat_serialization_roundtrip() {
        let original = RaftMessage::AppendEntries(AppendEntriesRequest::heartbeat(3, 1, 4, 2, 4));
        let bytes = original.to_bytes();
        let deserialized = RaftMessage::from_bytes(&bytes).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_invalid_message_deserialization() {
        // Empty message
        assert!(RaftMessage::from_bytes(&[]).is_err());
        
        // Unknown message type
        assert!(RaftMessage::from_bytes(&[99]).is_err());
        
        // Too short message
        assert!(RaftMessage::from_bytes(&[0, 1, 2]).is_err());
    }
}
