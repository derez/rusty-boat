//! Client request tracking for async response handling
//! 
//! This module provides functionality to track client requests that are waiting
//! for Raft log commitment before sending responses.

use crate::{Result, Error, NodeId, LogIndex};
use std::collections::HashMap;
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// Unique identifier for client requests
pub type RequestId = u64;

/// Information about a pending client request
#[derive(Debug)]
pub struct PendingRequest {
    /// Unique request ID
    pub request_id: RequestId,
    /// TCP stream to send response to
    pub stream: TcpStream,
    /// Log index where this request was appended
    pub log_index: LogIndex,
    /// When this request was received
    pub timestamp: Instant,
    /// The original request data for potential retry
    pub request_data: Vec<u8>,
    /// Number of retry attempts
    pub retry_count: u32,
}

impl PendingRequest {
    /// Create a new pending request
    pub fn new(
        request_id: RequestId,
        stream: TcpStream,
        log_index: LogIndex,
        request_data: Vec<u8>,
    ) -> Self {
        Self {
            request_id,
            stream,
            log_index,
            timestamp: Instant::now(),
            request_data,
            retry_count: 0,
        }
    }
    
    /// Check if this request has timed out
    pub fn is_timed_out(&self, timeout: Duration) -> bool {
        self.timestamp.elapsed() > timeout
    }
    
    /// Increment retry count
    pub fn increment_retry(&mut self) {
        self.retry_count += 1;
    }
}

/// Tracks client requests waiting for Raft log commitment
#[derive(Debug)]
pub struct ClientRequestTracker {
    /// Pending requests indexed by request ID
    pending_requests: HashMap<RequestId, PendingRequest>,
    /// Mapping from log index to request IDs
    log_index_to_requests: HashMap<LogIndex, Vec<RequestId>>,
    /// Next request ID to assign
    next_request_id: RequestId,
    /// Request timeout duration
    request_timeout: Duration,
    /// Maximum retry attempts
    max_retries: u32,
}

impl ClientRequestTracker {
    /// Create a new client request tracker
    pub fn new() -> Self {
        Self {
            pending_requests: HashMap::new(),
            log_index_to_requests: HashMap::new(),
            next_request_id: 1,
            request_timeout: Duration::from_secs(30), // 30 second timeout
            max_retries: 3,
        }
    }
    
    /// Create a new client request tracker with custom configuration
    pub fn with_config(request_timeout: Duration, max_retries: u32) -> Self {
        Self {
            pending_requests: HashMap::new(),
            log_index_to_requests: HashMap::new(),
            next_request_id: 1,
            request_timeout,
            max_retries,
        }
    }
    
    /// Add a new pending request
    pub fn add_request(
        &mut self,
        stream: TcpStream,
        log_index: LogIndex,
        request_data: Vec<u8>,
    ) -> RequestId {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        
        let pending_request = PendingRequest::new(request_id, stream, log_index, request_data);
        
        // Add to pending requests
        self.pending_requests.insert(request_id, pending_request);
        
        // Add to log index mapping
        self.log_index_to_requests
            .entry(log_index)
            .or_insert_with(Vec::new)
            .push(request_id);
        
        log::debug!(
            "Added pending client request {} for log index {}",
            request_id, log_index
        );
        
        request_id
    }
    
    /// Get requests that can be completed due to log commitment
    pub fn get_completable_requests(&mut self, commit_index: LogIndex) -> Vec<PendingRequest> {
        let mut completable = Vec::new();
        let mut indices_to_remove = Vec::new();
        
        // Find all log indices that are now committed
        for (&log_index, request_ids) in &self.log_index_to_requests {
            if log_index <= commit_index {
                // All requests for this log index can be completed
                for &request_id in request_ids {
                    if let Some(request) = self.pending_requests.remove(&request_id) {
                        log::info!(
                            "Request {} can be completed (log index {} committed)",
                            request_id, log_index
                        );
                        completable.push(request);
                    }
                }
                indices_to_remove.push(log_index);
            }
        }
        
        // Clean up completed indices
        for index in indices_to_remove {
            self.log_index_to_requests.remove(&index);
        }
        
        completable
    }
    
    /// Get requests that have timed out
    pub fn get_timed_out_requests(&mut self) -> Vec<PendingRequest> {
        let mut timed_out = Vec::new();
        let mut request_ids_to_remove = Vec::new();
        
        for (&request_id, request) in &self.pending_requests {
            if request.is_timed_out(self.request_timeout) {
                log::warn!(
                    "Request {} timed out after {:?} (log index {})",
                    request_id, self.request_timeout, request.log_index
                );
                request_ids_to_remove.push(request_id);
            }
        }
        
        // Remove timed out requests
        for request_id in request_ids_to_remove {
            if let Some(request) = self.pending_requests.remove(&request_id) {
                // Also remove from log index mapping
                if let Some(request_ids) = self.log_index_to_requests.get_mut(&request.log_index) {
                    request_ids.retain(|&id| id != request_id);
                    if request_ids.is_empty() {
                        self.log_index_to_requests.remove(&request.log_index);
                    }
                }
                timed_out.push(request);
            }
        }
        
        timed_out
    }
    
    /// Get requests that need to be retried due to leadership change
    pub fn get_requests_for_retry(&mut self) -> Vec<PendingRequest> {
        let mut requests_to_retry = Vec::new();
        
        // When leadership changes, all pending requests should be retried
        // For now, we'll return all pending requests
        for (request_id, mut request) in self.pending_requests.drain() {
            if request.retry_count < self.max_retries {
                request.increment_retry();
                log::info!(
                    "Request {} will be retried (attempt {}/{})",
                    request_id, request.retry_count, self.max_retries
                );
                requests_to_retry.push(request);
            } else {
                log::warn!(
                    "Request {} exceeded max retries ({}), will be failed",
                    request_id, self.max_retries
                );
                // Request will be dropped and client will get timeout
            }
        }
        
        // Clear log index mapping since we're retrying everything
        self.log_index_to_requests.clear();
        
        requests_to_retry
    }
    
    /// Get the number of pending requests
    pub fn pending_count(&self) -> usize {
        self.pending_requests.len()
    }
    
    /// Clear all pending requests (used when stepping down as leader)
    pub fn clear_all_requests(&mut self) -> Vec<PendingRequest> {
        let mut all_requests = Vec::new();
        
        for (_, request) in self.pending_requests.drain() {
            all_requests.push(request);
        }
        
        self.log_index_to_requests.clear();
        
        log::info!("Cleared {} pending client requests", all_requests.len());
        all_requests
    }
    
    /// Update configuration
    pub fn update_config(&mut self, request_timeout: Duration, max_retries: u32) {
        self.request_timeout = request_timeout;
        self.max_retries = max_retries;
    }
}

impl Default for ClientRequestTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::Duration;

    fn create_mock_stream() -> TcpStream {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        
        thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                // Just hold the connection open
                thread::sleep(Duration::from_millis(100));
            }
        });
        
        TcpStream::connect(addr).unwrap()
    }

    #[test]
    fn test_client_request_tracker_creation() {
        let tracker = ClientRequestTracker::new();
        assert_eq!(tracker.pending_count(), 0);
        assert_eq!(tracker.next_request_id, 1);
    }

    #[test]
    fn test_add_request() {
        let mut tracker = ClientRequestTracker::new();
        let stream = create_mock_stream();
        let request_data = b"test request".to_vec();
        
        let request_id = tracker.add_request(stream, 5, request_data.clone());
        
        assert_eq!(request_id, 1);
        assert_eq!(tracker.pending_count(), 1);
        assert_eq!(tracker.next_request_id, 2);
        
        // Check log index mapping
        assert!(tracker.log_index_to_requests.contains_key(&5));
        assert_eq!(tracker.log_index_to_requests[&5], vec![1]);
    }

    #[test]
    fn test_multiple_requests_same_log_index() {
        let mut tracker = ClientRequestTracker::new();
        let stream1 = create_mock_stream();
        let stream2 = create_mock_stream();
        
        let request_id1 = tracker.add_request(stream1, 5, b"request1".to_vec());
        let request_id2 = tracker.add_request(stream2, 5, b"request2".to_vec());
        
        assert_eq!(request_id1, 1);
        assert_eq!(request_id2, 2);
        assert_eq!(tracker.pending_count(), 2);
        
        // Both requests should be mapped to the same log index
        assert_eq!(tracker.log_index_to_requests[&5], vec![1, 2]);
    }

    #[test]
    fn test_get_completable_requests() {
        let mut tracker = ClientRequestTracker::new();
        let stream1 = create_mock_stream();
        let stream2 = create_mock_stream();
        let stream3 = create_mock_stream();
        
        // Add requests at different log indices
        tracker.add_request(stream1, 3, b"request1".to_vec());
        tracker.add_request(stream2, 5, b"request2".to_vec());
        tracker.add_request(stream3, 7, b"request3".to_vec());
        
        assert_eq!(tracker.pending_count(), 3);
        
        // Commit up to index 5
        let completable = tracker.get_completable_requests(5);
        
        // Should get requests for indices 3 and 5
        assert_eq!(completable.len(), 2);
        assert_eq!(tracker.pending_count(), 1); // Only request at index 7 remains
        
        // Check that the remaining request is at index 7
        assert!(tracker.log_index_to_requests.contains_key(&7));
        assert!(!tracker.log_index_to_requests.contains_key(&3));
        assert!(!tracker.log_index_to_requests.contains_key(&5));
    }

    #[test]
    fn test_timeout_handling() {
        let mut tracker = ClientRequestTracker::with_config(
            Duration::from_millis(50), // Very short timeout for testing
            3
        );
        
        let stream = create_mock_stream();
        tracker.add_request(stream, 5, b"request".to_vec());
        
        assert_eq!(tracker.pending_count(), 1);
        
        // Wait for timeout
        thread::sleep(Duration::from_millis(100));
        
        let timed_out = tracker.get_timed_out_requests();
        assert_eq!(timed_out.len(), 1);
        assert_eq!(tracker.pending_count(), 0);
    }

    #[test]
    fn test_retry_handling() {
        let mut tracker = ClientRequestTracker::new();
        let stream = create_mock_stream();
        
        tracker.add_request(stream, 5, b"request".to_vec());
        assert_eq!(tracker.pending_count(), 1);
        
        let retry_requests = tracker.get_requests_for_retry();
        assert_eq!(retry_requests.len(), 1);
        assert_eq!(retry_requests[0].retry_count, 1);
        assert_eq!(tracker.pending_count(), 0); // Requests are moved out for retry
    }

    #[test]
    fn test_max_retries() {
        let mut tracker = ClientRequestTracker::with_config(
            Duration::from_secs(30),
            2 // Max 2 retries
        );
        
        let stream = create_mock_stream();
        tracker.add_request(stream, 5, b"request".to_vec());
        
        // First retry
        let retry1 = tracker.get_requests_for_retry();
        assert_eq!(retry1.len(), 1);
        assert_eq!(retry1[0].retry_count, 1);
        
        // Add back for second retry
        let mut request = retry1.into_iter().next().unwrap();
        let request_id = tracker.add_request(request.stream, 5, request.request_data);
        tracker.pending_requests.get_mut(&request_id).unwrap().retry_count = 1;
        
        // Second retry
        let retry2 = tracker.get_requests_for_retry();
        assert_eq!(retry2.len(), 1);
        assert_eq!(retry2[0].retry_count, 2);
        
        // Add back for third retry attempt
        let mut request = retry2.into_iter().next().unwrap();
        let request_id = tracker.add_request(request.stream, 5, request.request_data);
        tracker.pending_requests.get_mut(&request_id).unwrap().retry_count = 2;
        
        // Third retry should fail (exceeds max retries)
        let retry3 = tracker.get_requests_for_retry();
        assert_eq!(retry3.len(), 0); // No requests returned, exceeded max retries
    }

    #[test]
    fn test_clear_all_requests() {
        let mut tracker = ClientRequestTracker::new();
        let stream1 = create_mock_stream();
        let stream2 = create_mock_stream();
        
        tracker.add_request(stream1, 3, b"request1".to_vec());
        tracker.add_request(stream2, 5, b"request2".to_vec());
        
        assert_eq!(tracker.pending_count(), 2);
        
        let cleared = tracker.clear_all_requests();
        assert_eq!(cleared.len(), 2);
        assert_eq!(tracker.pending_count(), 0);
        assert!(tracker.log_index_to_requests.is_empty());
    }
}
