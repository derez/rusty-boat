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
    /// Client request message
    ClientRequest(ClientRequest),
    /// Client response message
    ClientResponse(ClientResponse),
    /// Leader redirection message
    LeaderRedirect(LeaderRedirect),
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

/// Client request message for key-value operations
#[derive(Debug, Clone, PartialEq)]
pub struct ClientRequest {
    /// Unique request ID for tracking
    pub request_id: String,
    /// Client operation data (serialized KV operation)
    pub operation: Vec<u8>,
    /// Client ID for response routing
    pub client_id: NodeId,
}

impl ClientRequest {
    /// Create a new client request
    pub fn new(request_id: String, operation: Vec<u8>, client_id: NodeId) -> Self {
        Self {
            request_id,
            operation,
            client_id,
        }
    }
}

/// Client response message for operation results
#[derive(Debug, Clone, PartialEq)]
pub struct ClientResponse {
    /// Request ID this response corresponds to
    pub request_id: String,
    /// Success status of the operation
    pub success: bool,
    /// Response data (serialized result)
    pub data: Vec<u8>,
    /// Error message if operation failed
    pub error: Option<String>,
}

impl ClientResponse {
    /// Create a successful client response
    pub fn success(request_id: String, data: Vec<u8>) -> Self {
        Self {
            request_id,
            success: true,
            data,
            error: None,
        }
    }
    
    /// Create a failed client response
    pub fn error(request_id: String, error: String) -> Self {
        Self {
            request_id,
            success: false,
            data: Vec::new(),
            error: Some(error),
        }
    }
}

/// Leader redirection message for non-leader nodes
#[derive(Debug, Clone, PartialEq)]
pub struct LeaderRedirect {
    /// Current leader ID (if known)
    pub leader_id: Option<NodeId>,
    /// Current term
    pub term: Term,
    /// Message for client
    pub message: String,
}

impl LeaderRedirect {
    /// Create a new leader redirect message
    pub fn new(leader_id: Option<NodeId>, term: Term, message: String) -> Self {
        Self {
            leader_id,
            term,
            message,
        }
    }
    
    /// Create a redirect to known leader
    pub fn to_leader(leader_id: NodeId, term: Term) -> Self {
        Self::new(
            Some(leader_id),
            term,
            format!("Request must be sent to leader (Node {})", leader_id),
        )
    }
    
    /// Create a redirect when leader is unknown
    pub fn no_leader(term: Term) -> Self {
        Self::new(
            None,
            term,
            "No leader currently available, please retry".to_string(),
        )
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
            RaftMessage::ClientRequest(_) => 0, // Client requests don't have terms
            RaftMessage::ClientResponse(_) => 0, // Client responses don't have terms
            RaftMessage::LeaderRedirect(redirect) => redirect.term,
        }
    }
    
    /// Get the sender ID from any Raft message (if applicable)
    pub fn sender_id(&self) -> Option<NodeId> {
        match self {
            RaftMessage::RequestVote(req) => Some(req.candidate_id),
            RaftMessage::RequestVoteResponse(resp) => Some(resp.voter_id),
            RaftMessage::AppendEntries(req) => Some(req.leader_id),
            RaftMessage::AppendEntriesResponse(resp) => Some(resp.follower_id),
            RaftMessage::ClientRequest(req) => Some(req.client_id),
            RaftMessage::ClientResponse(_) => None, // Client responses don't have a sender ID
            RaftMessage::LeaderRedirect(redirect) => redirect.leader_id,
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
            RaftMessage::ClientRequest(req) => {
                let mut bytes = vec![4u8]; // Message type
                bytes.extend_from_slice(&(req.request_id.len() as u32).to_be_bytes());
                bytes.extend_from_slice(req.request_id.as_bytes());
                bytes.extend_from_slice(&(req.operation.len() as u32).to_be_bytes());
                bytes.extend_from_slice(&req.operation);
                bytes.extend_from_slice(&req.client_id.to_be_bytes());
                bytes
            }
            RaftMessage::ClientResponse(resp) => {
                let mut bytes = vec![5u8]; // Message type
                bytes.extend_from_slice(&(resp.request_id.len() as u32).to_be_bytes());
                bytes.extend_from_slice(resp.request_id.as_bytes());
                bytes.push(if resp.success { 1 } else { 0 });
                bytes.extend_from_slice(&(resp.data.len() as u32).to_be_bytes());
                bytes.extend_from_slice(&resp.data);
                if let Some(ref error) = resp.error {
                    bytes.push(1); // Has error
                    bytes.extend_from_slice(&(error.len() as u32).to_be_bytes());
                    bytes.extend_from_slice(error.as_bytes());
                } else {
                    bytes.push(0); // No error
                }
                bytes
            }
            RaftMessage::LeaderRedirect(redirect) => {
                let mut bytes = vec![6u8]; // Message type
                if let Some(leader_id) = redirect.leader_id {
                    bytes.push(1); // Has leader ID
                    bytes.extend_from_slice(&leader_id.to_be_bytes());
                } else {
                    bytes.push(0); // No leader ID
                }
                bytes.extend_from_slice(&redirect.term.to_be_bytes());
                bytes.extend_from_slice(&(redirect.message.len() as u32).to_be_bytes());
                bytes.extend_from_slice(redirect.message.as_bytes());
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
            4 => {
                // ClientRequest
                if bytes.len() < 17 { // 1 + 4 + 4 + 8 minimum
                    return Err(crate::Error::Network("ClientRequest message too short".to_string()));
                }
                
                let request_id_len = u32::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                ]) as usize;
                offset += 4;
                
                if offset + request_id_len > bytes.len() {
                    return Err(crate::Error::Network("ClientRequest request_id too short".to_string()));
                }
                
                let request_id = String::from_utf8(bytes[offset..offset + request_id_len].to_vec())
                    .map_err(|_| crate::Error::Network("Invalid UTF-8 in request_id".to_string()))?;
                offset += request_id_len;
                
                if offset + 4 > bytes.len() {
                    return Err(crate::Error::Network("ClientRequest operation length missing".to_string()));
                }
                
                let operation_len = u32::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                ]) as usize;
                offset += 4;
                
                if offset + operation_len > bytes.len() {
                    return Err(crate::Error::Network("ClientRequest operation too short".to_string()));
                }
                
                let operation = bytes[offset..offset + operation_len].to_vec();
                offset += operation_len;
                
                if offset + 8 > bytes.len() {
                    return Err(crate::Error::Network("ClientRequest client_id missing".to_string()));
                }
                
                let client_id = u64::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                    bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                ]);
                
                Ok(RaftMessage::ClientRequest(ClientRequest::new(
                    request_id, operation, client_id
                )))
            }
            5 => {
                // ClientResponse
                if bytes.len() < 10 { // 1 + 4 + 1 + 4 + 1 minimum
                    return Err(crate::Error::Network("ClientResponse message too short".to_string()));
                }
                
                let request_id_len = u32::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                ]) as usize;
                offset += 4;
                
                if offset + request_id_len > bytes.len() {
                    return Err(crate::Error::Network("ClientResponse request_id too short".to_string()));
                }
                
                let request_id = String::from_utf8(bytes[offset..offset + request_id_len].to_vec())
                    .map_err(|_| crate::Error::Network("Invalid UTF-8 in request_id".to_string()))?;
                offset += request_id_len;
                
                if offset + 1 > bytes.len() {
                    return Err(crate::Error::Network("ClientResponse success flag missing".to_string()));
                }
                
                let success = bytes[offset] != 0;
                offset += 1;
                
                if offset + 4 > bytes.len() {
                    return Err(crate::Error::Network("ClientResponse data length missing".to_string()));
                }
                
                let data_len = u32::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                ]) as usize;
                offset += 4;
                
                if offset + data_len > bytes.len() {
                    return Err(crate::Error::Network("ClientResponse data too short".to_string()));
                }
                
                let data = bytes[offset..offset + data_len].to_vec();
                offset += data_len;
                
                if offset + 1 > bytes.len() {
                    return Err(crate::Error::Network("ClientResponse error flag missing".to_string()));
                }
                
                let has_error = bytes[offset] != 0;
                offset += 1;
                
                let error = if has_error {
                    if offset + 4 > bytes.len() {
                        return Err(crate::Error::Network("ClientResponse error length missing".to_string()));
                    }
                    
                    let error_len = u32::from_be_bytes([
                        bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                    ]) as usize;
                    offset += 4;
                    
                    if offset + error_len > bytes.len() {
                        return Err(crate::Error::Network("ClientResponse error too short".to_string()));
                    }
                    
                    let error_str = String::from_utf8(bytes[offset..offset + error_len].to_vec())
                        .map_err(|_| crate::Error::Network("Invalid UTF-8 in error".to_string()))?;
                    Some(error_str)
                } else {
                    None
                };
                
                Ok(RaftMessage::ClientResponse(ClientResponse {
                    request_id,
                    success,
                    data,
                    error,
                }))
            }
            6 => {
                // LeaderRedirect
                if bytes.len() < 14 { // 1 + 1 + 8 + 4 minimum
                    return Err(crate::Error::Network("LeaderRedirect message too short".to_string()));
                }
                
                let has_leader = bytes[offset] != 0;
                offset += 1;
                
                let leader_id = if has_leader {
                    if offset + 8 > bytes.len() {
                        return Err(crate::Error::Network("LeaderRedirect leader_id missing".to_string()));
                    }
                    
                    let id = u64::from_be_bytes([
                        bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                        bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                    ]);
                    offset += 8;
                    Some(id)
                } else {
                    None
                };
                
                if offset + 8 > bytes.len() {
                    return Err(crate::Error::Network("LeaderRedirect term missing".to_string()));
                }
                
                let term = u64::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                    bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7],
                ]);
                offset += 8;
                
                if offset + 4 > bytes.len() {
                    return Err(crate::Error::Network("LeaderRedirect message length missing".to_string()));
                }
                
                let message_len = u32::from_be_bytes([
                    bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3],
                ]) as usize;
                offset += 4;
                
                if offset + message_len > bytes.len() {
                    return Err(crate::Error::Network("LeaderRedirect message too short".to_string()));
                }
                
                let message = String::from_utf8(bytes[offset..offset + message_len].to_vec())
                    .map_err(|_| crate::Error::Network("Invalid UTF-8 in message".to_string()))?;
                
                Ok(RaftMessage::LeaderRedirect(LeaderRedirect::new(
                    leader_id, term, message
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
