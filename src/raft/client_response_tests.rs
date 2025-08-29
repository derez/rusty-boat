//! Tests for client response handling functionality
//! 
//! This module tests the async client response handling system that waits
//! for Raft log commitment before sending responses to clients.

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::storage::log_storage::InMemoryLogStorage;
    use crate::storage::state_storage::InMemoryStateStorage;
    use crate::network::transport::MockTransport;
    use crate::kv::KVOperation;
    use std::net::{TcpListener, TcpStream};
    use std::io::Write;

    // Helper function to create a mock TCP stream for testing
    fn create_mock_tcp_stream() -> TcpStream {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        
        // Create a connection in a separate thread
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                // Write a simple response to keep the connection alive
                let _ = stream.write_all(b"test");
            }
        });
        
        // Connect to the listener
        TcpStream::connect(addr).unwrap()
    }

    #[test]
    fn test_client_tracker_initialization() {
        let config = RaftConfig::fast(0, vec![0]);
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(0));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Initially no client tracker
        assert_eq!(node.pending_client_requests_count(), 0);
        
        // Become leader - should initialize client tracker
        node.transition_to(NodeState::Leader).unwrap();
        assert_eq!(node.pending_client_requests_count(), 0); // Still 0 but tracker is initialized
    }

    #[test]
    fn test_track_client_request_leader_only() {
        let config = RaftConfig::fast(0, vec![0, 1, 2]);
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(0));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Followers cannot track client requests
        let stream = create_mock_tcp_stream();
        let result = node.track_client_request(stream, 1, b"test_request".to_vec());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not the leader"));
        
        // Become leader
        node.transition_to(NodeState::Leader).unwrap();
        
        // Now should be able to track requests
        let stream = create_mock_tcp_stream();
        let request_id = node.track_client_request(stream, 1, b"test_request".to_vec()).unwrap();
        assert_eq!(request_id, 1);
        assert_eq!(node.pending_client_requests_count(), 1);
    }

    #[test]
    fn test_client_command_integration() {
        let config = RaftConfig::fast(0, vec![0]);
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(0));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Become leader
        node.transition_to(NodeState::Leader).unwrap();
        
        // Create a KV operation
        let put_operation = KVOperation::Put {
            key: "test_key".to_string(),
            value: b"test_value".to_vec(),
        };
        let operation_data = put_operation.to_bytes();
        
        // Append client command to Raft log
        let log_index = node.append_client_command(operation_data.clone()).unwrap();
        assert_eq!(log_index, 1);
        
        // Track the client request
        let stream = create_mock_tcp_stream();
        let request_id = node.track_client_request(stream, log_index, operation_data).unwrap();
        assert_eq!(request_id, 1);
        assert_eq!(node.pending_client_requests_count(), 1);
        
        // Simulate log commitment (single node cluster commits immediately)
        node.advance_commit_index().unwrap();
        
        // In a single node cluster, the entry should be committed immediately
        assert_eq!(node.get_commit_index(), 1);
        // Note: last_applied is updated internally when commit_index advances
    }

    #[test]
    fn test_leader_state_transition_clears_tracker() {
        let config = RaftConfig::fast(0, vec![0, 1, 2]);
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(0));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Become leader and track a request
        node.transition_to(NodeState::Leader).unwrap();
        let stream = create_mock_tcp_stream();
        let _request_id = node.track_client_request(stream, 1, b"test_request".to_vec()).unwrap();
        assert_eq!(node.pending_client_requests_count(), 1);
        
        // Step down to follower - client tracker should be cleared
        node.transition_to(NodeState::Follower).unwrap();
        assert_eq!(node.pending_client_requests_count(), 0);
    }

    #[test]
    fn test_multiple_client_requests() {
        let config = RaftConfig::fast(0, vec![0]);
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(0));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Become leader
        node.transition_to(NodeState::Leader).unwrap();
        
        // Track multiple requests
        let stream1 = create_mock_tcp_stream();
        let stream2 = create_mock_tcp_stream();
        let stream3 = create_mock_tcp_stream();
        let request_id1 = node.track_client_request(stream1, 1, b"request1".to_vec()).unwrap();
        let request_id2 = node.track_client_request(stream2, 2, b"request2".to_vec()).unwrap();
        let request_id3 = node.track_client_request(stream3, 3, b"request3".to_vec()).unwrap();
        
        assert_eq!(request_id1, 1);
        assert_eq!(request_id2, 2);
        assert_eq!(request_id3, 3);
        assert_eq!(node.pending_client_requests_count(), 3);
    }

    #[test]
    fn test_client_request_with_kv_operations() {
        let config = RaftConfig::fast(0, vec![0]);
        let state_storage = Box::new(InMemoryStateStorage::new());
        let log_storage = Box::new(InMemoryLogStorage::new());
        let transport = Box::new(MockTransport::new(0));
        
        let mut node = RaftNode::with_dependencies(config, state_storage, log_storage, transport).unwrap();
        
        // Become leader
        node.transition_to(NodeState::Leader).unwrap();
        
        // Test PUT operation
        let put_op = KVOperation::Put {
            key: "key1".to_string(),
            value: b"value1".to_vec(),
        };
        let put_data = put_op.to_bytes();
        let log_index1 = node.append_client_command(put_data.clone()).unwrap();
        let stream1 = create_mock_tcp_stream();
        let _request_id1 = node.track_client_request(stream1, log_index1, put_data).unwrap();
        
        // Test DELETE operation
        let delete_op = KVOperation::Delete {
            key: "key2".to_string(),
        };
        let delete_data = delete_op.to_bytes();
        let log_index2 = node.append_client_command(delete_data.clone()).unwrap();
        let stream2 = create_mock_tcp_stream();
        let _request_id2 = node.track_client_request(stream2, log_index2, delete_data).unwrap();
        
        assert_eq!(node.pending_client_requests_count(), 2);
        assert_eq!(log_index1, 1);
        assert_eq!(log_index2, 2);
        
        // Verify log entries were created
        let entry1 = node.get_log_entry(1).unwrap();
        let entry2 = node.get_log_entry(2).unwrap();
        
        // Verify we can deserialize the operations back
        let recovered_put = KVOperation::from_bytes(&entry1.data).unwrap();
        let recovered_delete = KVOperation::from_bytes(&entry2.data).unwrap();
        
        match recovered_put {
            KVOperation::Put { key, value } => {
                assert_eq!(key, "key1");
                assert_eq!(value, b"value1");
            }
            _ => panic!("Expected PUT operation"),
        }
        
        match recovered_delete {
            KVOperation::Delete { key } => {
                assert_eq!(key, "key2");
            }
            _ => panic!("Expected DELETE operation"),
        }
    }
}
