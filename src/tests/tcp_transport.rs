//! TCP Transport Integration Tests
//! 
//! This module contains integration tests for the TCP transport implementation
//! to verify actual network communication between nodes.

use crate::network::{NetworkConfig, NodeAddress, TcpTransport, NetworkTransport};
use crate::raft::messages::{RaftMessage, RequestVoteRequest, AppendEntriesRequest};
use crate::storage::LogEntry;
use crate::{NodeId, Result};
use std::collections::HashMap;
use std::thread;
use std::time::Duration;

/// Test basic TCP transport creation and configuration
#[test]
fn test_tcp_transport_creation() {
    let local_addr = NodeAddress::new(1, "127.0.0.1".to_string(), 8001);
    let mut config = NetworkConfig::new(local_addr);
    config.add_node(NodeAddress::new(2, "127.0.0.1".to_string(), 8002));
    
    let transport = TcpTransport::new(config, 1, "127.0.0.1:8001");
    assert!(transport.is_ok(), "Failed to create TCP transport: {:?}", transport.err());
    
    let transport = transport.unwrap();
    assert_eq!(transport.connected_nodes().len(), 0); // No connections initially
}

/// Test TCP transport message sending and receiving with RaftMessage serialization
#[test]
fn test_tcp_transport_raft_message_communication() {
    // Set up two nodes with different ports
    let local_addr1 = NodeAddress::new(1, "127.0.0.1".to_string(), 8011);
    let mut config1 = NetworkConfig::new(local_addr1);
    config1.add_node(NodeAddress::new(2, "127.0.0.1".to_string(), 8012));
    
    let local_addr2 = NodeAddress::new(2, "127.0.0.1".to_string(), 8012);
    let mut config2 = NetworkConfig::new(local_addr2);
    config2.add_node(NodeAddress::new(1, "127.0.0.1".to_string(), 8011));
    
    // Create transports
    let transport1 = TcpTransport::new(config1, 1, "127.0.0.1:8011").unwrap();
    let transport2 = TcpTransport::new(config2, 2, "127.0.0.1:8012").unwrap();
    
    // Start listeners
    transport1.start_listener().unwrap();
    transport2.start_listener().unwrap();
    
    // Give listeners time to start
    thread::sleep(Duration::from_millis(100));
    
    // Create a RequestVote message
    let vote_request = RequestVoteRequest::new(5, 1, 10, 4);
    let raft_message = RaftMessage::RequestVote(vote_request);
    let message_bytes = raft_message.to_bytes();
    
    // Send message from node 1 to node 2
    let send_result = transport1.send_message(2, message_bytes.clone());
    assert!(send_result.is_ok(), "Failed to send message: {:?}", send_result.err());
    
    // Give time for message to be received
    thread::sleep(Duration::from_millis(200));
    
    // Check if node 2 received the message
    let received_messages = transport2.receive_messages();
    assert_eq!(received_messages.len(), 1, "Expected 1 message, got {}", received_messages.len());
    
    let (sender_id, received_bytes) = &received_messages[0];
    assert_eq!(*sender_id, 1, "Expected sender ID 1, got {}", sender_id);
    
    // Deserialize and verify the message
    let received_message = RaftMessage::from_bytes(received_bytes).unwrap();
    assert_eq!(received_message, raft_message, "Message content mismatch");
    
    println!("TCP transport: successfully sent and received RaftMessage");
}

/// Test TCP transport with AppendEntries message containing log entries
#[test]
fn test_tcp_transport_append_entries_communication() {
    // Set up two nodes with different ports
    let local_addr1 = NodeAddress::new(1, "127.0.0.1".to_string(), 8021);
    let mut config1 = NetworkConfig::new(local_addr1);
    config1.add_node(NodeAddress::new(2, "127.0.0.1".to_string(), 8022));
    
    let local_addr2 = NodeAddress::new(2, "127.0.0.1".to_string(), 8022);
    let mut config2 = NetworkConfig::new(local_addr2);
    config2.add_node(NodeAddress::new(1, "127.0.0.1".to_string(), 8021));
    
    // Create transports
    let transport1 = TcpTransport::new(config1, 1, "127.0.0.1:8021").unwrap();
    let transport2 = TcpTransport::new(config2, 2, "127.0.0.1:8022").unwrap();
    
    // Start listeners
    transport1.start_listener().unwrap();
    transport2.start_listener().unwrap();
    
    // Give listeners time to start
    thread::sleep(Duration::from_millis(100));
    
    // Create an AppendEntries message with log entries
    let entries = vec![
        LogEntry::new(3, 5, b"key1=value1".to_vec()),
        LogEntry::new(3, 6, b"key2=value2".to_vec()),
    ];
    
    let append_request = AppendEntriesRequest::new(3, 1, 4, 2, entries, 4);
    let raft_message = RaftMessage::AppendEntries(append_request);
    let message_bytes = raft_message.to_bytes();
    
    println!("TCP transport: sending AppendEntries message ({} bytes)", message_bytes.len());
    
    // Send message from node 1 to node 2
    let send_result = transport1.send_message(2, message_bytes.clone());
    assert!(send_result.is_ok(), "Failed to send message: {:?}", send_result.err());
    
    // Give time for message to be received
    thread::sleep(Duration::from_millis(200));
    
    // Check if node 2 received the message
    let received_messages = transport2.receive_messages();
    assert_eq!(received_messages.len(), 1, "Expected 1 message, got {}", received_messages.len());
    
    let (sender_id, received_bytes) = &received_messages[0];
    assert_eq!(*sender_id, 1, "Expected sender ID 1, got {}", sender_id);
    
    // Deserialize and verify the message
    let received_message = RaftMessage::from_bytes(received_bytes).unwrap();
    assert_eq!(received_message, raft_message, "Message content mismatch");
    
    // Verify the log entries were preserved
    if let RaftMessage::AppendEntries(req) = received_message {
        assert_eq!(req.entries.len(), 2, "Expected 2 log entries");
        assert_eq!(req.entries[0].data, b"key1=value1");
        assert_eq!(req.entries[1].data, b"key2=value2");
    } else {
        panic!("Expected AppendEntries message");
    }
    
    println!("TCP transport: successfully sent and received AppendEntries with log entries");
}

/// Test TCP transport bidirectional communication
#[test]
fn test_tcp_transport_bidirectional_communication() {
    // Set up two nodes with different ports
    let local_addr1 = NodeAddress::new(1, "127.0.0.1".to_string(), 8031);
    let mut config1 = NetworkConfig::new(local_addr1);
    config1.add_node(NodeAddress::new(2, "127.0.0.1".to_string(), 8032));
    
    let local_addr2 = NodeAddress::new(2, "127.0.0.1".to_string(), 8032);
    let mut config2 = NetworkConfig::new(local_addr2);
    config2.add_node(NodeAddress::new(1, "127.0.0.1".to_string(), 8031));
    
    // Create transports
    let transport1 = TcpTransport::new(config1, 1, "127.0.0.1:8031").unwrap();
    let transport2 = TcpTransport::new(config2, 2, "127.0.0.1:8032").unwrap();
    
    // Start listeners
    transport1.start_listener().unwrap();
    transport2.start_listener().unwrap();
    
    // Give listeners time to start
    thread::sleep(Duration::from_millis(100));
    
    // Create messages for both directions
    let vote_request = RequestVoteRequest::new(5, 1, 10, 4);
    let message1to2 = RaftMessage::RequestVote(vote_request);
    
    let heartbeat = AppendEntriesRequest::heartbeat(5, 2, 10, 4, 8);
    let message2to1 = RaftMessage::AppendEntries(heartbeat);
    
    // Send message from node 1 to node 2
    let send_result = transport1.send_message(2, message1to2.to_bytes());
    assert!(send_result.is_ok(), "Failed to send message 1->2: {:?}", send_result.err());
    
    // Send message from node 2 to node 1
    let send_result = transport2.send_message(1, message2to1.to_bytes());
    assert!(send_result.is_ok(), "Failed to send message 2->1: {:?}", send_result.err());
    
    // Give time for messages to be received
    thread::sleep(Duration::from_millis(200));
    
    // Check if node 2 received message from node 1
    let received_messages2 = transport2.receive_messages();
    assert_eq!(received_messages2.len(), 1, "Node 2 expected 1 message, got {}", received_messages2.len());
    
    let (sender_id, received_bytes) = &received_messages2[0];
    assert_eq!(*sender_id, 1, "Expected sender ID 1, got {}", sender_id);
    let received_message = RaftMessage::from_bytes(received_bytes).unwrap();
    assert_eq!(received_message, message1to2, "Message 1->2 content mismatch");
    
    // Check if node 1 received message from node 2
    let received_messages1 = transport1.receive_messages();
    assert_eq!(received_messages1.len(), 1, "Node 1 expected 1 message, got {}", received_messages1.len());
    
    let (sender_id, received_bytes) = &received_messages1[0];
    assert_eq!(*sender_id, 2, "Expected sender ID 2, got {}", sender_id);
    let received_message = RaftMessage::from_bytes(received_bytes).unwrap();
    assert_eq!(received_message, message2to1, "Message 2->1 content mismatch");
    
    println!("TCP transport: successfully completed bidirectional communication");
}

/// Test TCP transport error handling for invalid addresses
#[test]
fn test_tcp_transport_error_handling() {
    let local_addr = NodeAddress::new(1, "127.0.0.1".to_string(), 8041);
    let mut config = NetworkConfig::new(local_addr);
    config.add_node(NodeAddress::new(2, "127.0.0.1".to_string(), 8042)); // This node won't be listening
    
    let transport = TcpTransport::new(config, 1, "127.0.0.1:8041").unwrap();
    transport.start_listener().unwrap();
    
    // Give listener time to start
    thread::sleep(Duration::from_millis(100));
    
    // Try to send message to non-listening node
    let vote_request = RequestVoteRequest::new(5, 1, 10, 4);
    let message = RaftMessage::RequestVote(vote_request);
    let send_result = transport.send_message(2, message.to_bytes());
    
    // Should fail because node 2 is not listening
    assert!(send_result.is_err(), "Expected send to fail, but it succeeded");
    
    println!("TCP transport: correctly handled connection error");
}

/// Test TCP transport with multiple rapid messages
#[test]
fn test_tcp_transport_multiple_messages() {
    // Set up two nodes with different ports
    let local_addr1 = NodeAddress::new(1, "127.0.0.1".to_string(), 8051);
    let mut config1 = NetworkConfig::new(local_addr1);
    config1.add_node(NodeAddress::new(2, "127.0.0.1".to_string(), 8052));
    
    let local_addr2 = NodeAddress::new(2, "127.0.0.1".to_string(), 8052);
    let mut config2 = NetworkConfig::new(local_addr2);
    config2.add_node(NodeAddress::new(1, "127.0.0.1".to_string(), 8051));
    
    // Create transports
    let transport1 = TcpTransport::new(config1, 1, "127.0.0.1:8051").unwrap();
    let transport2 = TcpTransport::new(config2, 2, "127.0.0.1:8052").unwrap();
    
    // Start listeners
    transport1.start_listener().unwrap();
    transport2.start_listener().unwrap();
    
    // Give listeners time to start
    thread::sleep(Duration::from_millis(100));
    
    // Send multiple messages rapidly
    let message_count = 5usize;
    for i in 0..message_count {
        let vote_request = RequestVoteRequest::new((i + 1) as u64, 1, 10, 4);
        let message = RaftMessage::RequestVote(vote_request);
        let send_result = transport1.send_message(2, message.to_bytes());
        assert!(send_result.is_ok(), "Failed to send message {}: {:?}", i, send_result.err());
        
        // Small delay between messages
        thread::sleep(Duration::from_millis(10));
    }
    
    // Give time for all messages to be received
    thread::sleep(Duration::from_millis(300));
    
    // Check if all messages were received
    let received_messages = transport2.receive_messages();
    assert_eq!(received_messages.len(), message_count, 
               "Expected {} messages, got {}", message_count, received_messages.len());
    
    // Verify all messages were received correctly
    for (i, (sender_id, received_bytes)) in received_messages.iter().enumerate() {
        assert_eq!(*sender_id, 1, "Expected sender ID 1 for message {}, got {}", i, sender_id);
        
        let received_message = RaftMessage::from_bytes(received_bytes).unwrap();
        if let RaftMessage::RequestVote(req) = received_message {
            assert_eq!(req.term, (i + 1) as u64, "Expected term {} for message {}, got {}", i + 1, i, req.term);
        } else {
            panic!("Expected RequestVote message for message {}", i);
        }
    }
    
    println!("TCP transport: successfully sent and received {} messages", message_count);
}
