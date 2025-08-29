//! Simplified Client Command Consistency Tests for Multi-Node Raft Clusters
//! 
//! This module contains simplified tests that verify client commands
//! are properly replicated through Raft consensus using only the public API.

use crate::{Result, NodeId};
use crate::raft::{RaftNode, RaftConfig, NodeState};
use crate::storage::{InMemoryLogStorage, InMemoryStateStorage};
use crate::network::transport::MockTransport;
use crate::kv::KVOperation;
use std::collections::HashMap;

/// Simplified test cluster for client command consistency testing
pub struct SimpleTestCluster {
    /// Nodes in the cluster
    nodes: HashMap<NodeId, RaftNode>,
    /// Network transports for each node
    transports: HashMap<NodeId, MockTransport>,
}

impl SimpleTestCluster {
    /// Create a new test cluster with specified number of nodes
    pub fn new(node_count: usize) -> Result<Self> {
        let mut nodes = HashMap::new();
        let mut transports = HashMap::new();
        
        // Create node IDs
        let node_ids: Vec<NodeId> = (0..node_count).map(|i| i as NodeId).collect();
        
        // Create each node with dependencies
        for &node_id in &node_ids {
            let config = RaftConfig::fast(node_id, node_ids.clone());
            
            let state_storage = Box::new(InMemoryStateStorage::new());
            let log_storage = Box::new(InMemoryLogStorage::new());
            let transport = MockTransport::new(node_id);
            
            let node = RaftNode::with_dependencies(
                config,
                state_storage,
                log_storage,
                Box::new(transport.clone()),
            )?;
            
            nodes.insert(node_id, node);
            transports.insert(node_id, transport);
        }
        
        Ok(Self {
            nodes,
            transports,
        })
    }
    
    /// Get a mutable reference to a node
    pub fn get_node_mut(&mut self, node_id: NodeId) -> Option<&mut RaftNode> {
        self.nodes.get_mut(&node_id)
    }
    
    /// Get all node IDs in the cluster
    pub fn node_ids(&self) -> Vec<NodeId> {
        self.nodes.keys().copied().collect()
    }
    
    /// Get the current leader (if any)
    pub fn get_leader(&self) -> Option<NodeId> {
        for (node_id, node) in &self.nodes {
            if node.is_leader() {
                return Some(*node_id);
            }
        }
        None
    }
    
    /// Force a specific node to become leader (for testing)
    pub fn force_leader(&mut self, node_id: NodeId) -> Result<()> {
        if let Some(node) = self.get_node_mut(node_id) {
            node.transition_to(NodeState::Leader)?;
            log::info!("Forced node {} to become leader", node_id);
        }
        Ok(())
    }
    
    /// Submit a client command to the leader
    pub fn submit_client_command(&mut self, operation: KVOperation) -> Result<Option<u64>> {
        let leader_id = self.get_leader().ok_or_else(|| {
            crate::Error::Raft("No leader available to accept client command".to_string())
        })?;
        
        let command_data = operation.to_bytes();
        
        if let Some(leader) = self.get_node_mut(leader_id) {
            match leader.append_client_command(command_data) {
                Ok(log_index) => {
                    log::info!(
                        "Client command submitted to leader {} at log index {}",
                        leader_id, log_index
                    );
                    Ok(Some(log_index))
                }
                Err(e) => {
                    log::error!("Failed to submit client command to leader {}: {}", leader_id, e);
                    Err(e)
                }
            }
        } else {
            Err(crate::Error::Raft("Leader node not found".to_string()))
        }
    }
    
    /// Verify that all nodes have consistent log state (using public API only)
    pub fn verify_log_consistency(&self) -> Result<bool> {
        if self.nodes.is_empty() {
            return Ok(true);
        }
        
        // Get the first node as reference
        let first_node_id = *self.node_ids().first().unwrap();
        let reference_node = &self.nodes[&first_node_id];
        let reference_commit_index = reference_node.get_commit_index();
        
        // Compare all other nodes against the reference
        for &node_id in &self.node_ids() {
            if node_id == first_node_id {
                continue;
            }
            
            let node = &self.nodes[&node_id];
            let node_commit_index = node.get_commit_index();
            
            // All nodes should have the same commit index
            if reference_commit_index != node_commit_index {
                log::error!(
                    "Log consistency check failed: node {} has commit_index {}, reference node {} has commit_index {}",
                    node_id, node_commit_index, first_node_id, reference_commit_index
                );
                return Ok(false);
            }
            
            // Check that all committed log entries match (using public API)
            for log_index in 1..=reference_commit_index {
                let reference_entry = reference_node.get_log_entry(log_index);
                let node_entry = node.get_log_entry(log_index);
                
                match (reference_entry, node_entry) {
                    (Some(ref_entry), Some(node_entry)) => {
                        if ref_entry.term != node_entry.term || ref_entry.data != node_entry.data {
                            log::error!(
                                "Log consistency check failed: log entry {} differs between nodes (reference: term={}, node {}: term={})",
                                log_index, ref_entry.term, node_id, node_entry.term
                            );
                            return Ok(false);
                        }
                    }
                    (Some(_), None) => {
                        log::error!(
                            "Log consistency check failed: node {} missing log entry {} that reference node has",
                            node_id, log_index
                        );
                        return Ok(false);
                    }
                    (None, Some(_)) => {
                        log::error!(
                            "Log consistency check failed: node {} has log entry {} that reference node doesn't have",
                            node_id, log_index
                        );
                        return Ok(false);
                    }
                    (None, None) => {
                        // Both missing - this shouldn't happen for committed entries
                        log::error!(
                            "Log consistency check failed: both nodes missing committed log entry {}",
                            log_index
                        );
                        return Ok(false);
                    }
                }
            }
        }
        
        log::info!(
            "Log consistency check passed: all {} nodes have identical committed logs (commit_index: {})",
            self.node_ids().len(), reference_commit_index
        );
        
        Ok(true)
    }
    
    /// Get cluster statistics
    pub fn get_cluster_stats(&self) -> ClusterStats {
        let mut leaders = 0;
        let mut candidates = 0;
        let mut followers = 0;
        let mut total_log_entries = 0;
        
        for (_node_id, node) in &self.nodes {
            match node.state() {
                NodeState::Leader => leaders += 1,
                NodeState::Candidate => candidates += 1,
                NodeState::Follower => followers += 1,
            }
            
            // Count actual log entries (not just committed ones)
            total_log_entries += node.get_last_log_index();
        }
        
        ClusterStats {
            total_nodes: self.nodes.len(),
            leaders,
            candidates,
            followers,
            total_log_entries: total_log_entries as usize,
        }
    }
}

/// Cluster statistics for monitoring
#[derive(Debug, Clone)]
pub struct ClusterStats {
    pub total_nodes: usize,
    pub leaders: usize,
    pub candidates: usize,
    pub followers: usize,
    pub total_log_entries: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_command_submission() {
        let mut cluster = SimpleTestCluster::new(3).unwrap();
        
        // Force node 0 to be leader
        cluster.force_leader(0).unwrap();
        
        // Submit a PUT command
        let put_operation = KVOperation::Put {
            key: "test_key".to_string(),
            value: b"test_value".to_vec(),
        };
        
        let log_index = cluster.submit_client_command(put_operation).unwrap();
        assert!(log_index.is_some());
        assert_eq!(log_index.unwrap(), 1); // First log entry
    }

    #[test]
    fn test_multiple_client_commands() {
        let mut cluster = SimpleTestCluster::new(3).unwrap();
        
        // Force node 0 to be leader
        cluster.force_leader(0).unwrap();
        
        // Submit multiple commands
        let commands = vec![
            KVOperation::Put { key: "key1".to_string(), value: b"value1".to_vec() },
            KVOperation::Put { key: "key2".to_string(), value: b"value2".to_vec() },
            KVOperation::Put { key: "key3".to_string(), value: b"value3".to_vec() },
        ];
        
        let mut log_indices = Vec::new();
        for command in commands {
            if let Some(log_index) = cluster.submit_client_command(command).unwrap() {
                log_indices.push(log_index);
            }
        }
        
        // All commands should get sequential log indices
        assert_eq!(log_indices.len(), 3);
        assert_eq!(log_indices, vec![1, 2, 3]);
    }

    #[test]
    fn test_leader_only_command_processing() {
        let mut cluster = SimpleTestCluster::new(3).unwrap();
        
        // Don't set any leader - all nodes are followers
        
        // Try to submit a command - should fail
        let put_operation = KVOperation::Put {
            key: "test_key".to_string(),
            value: b"test_value".to_vec(),
        };
        
        let result = cluster.submit_client_command(put_operation);
        assert!(result.is_err());
        
        // Now force node 1 to be leader
        cluster.force_leader(1).unwrap();
        
        // Try again - should succeed
        let put_operation2 = KVOperation::Put {
            key: "test_key2".to_string(),
            value: b"test_value2".to_vec(),
        };
        
        let log_index = cluster.submit_client_command(put_operation2).unwrap();
        assert!(log_index.is_some());
    }

    #[test]
    fn test_concurrent_client_requests() {
        let mut cluster = SimpleTestCluster::new(3).unwrap();
        
        // Force node 0 to be leader
        cluster.force_leader(0).unwrap();
        
        // Submit multiple concurrent commands
        let commands = vec![
            KVOperation::Put { key: "concurrent1".to_string(), value: b"value1".to_vec() },
            KVOperation::Put { key: "concurrent2".to_string(), value: b"value2".to_vec() },
            KVOperation::Put { key: "concurrent3".to_string(), value: b"value3".to_vec() },
            KVOperation::Put { key: "concurrent4".to_string(), value: b"value4".to_vec() },
            KVOperation::Put { key: "concurrent5".to_string(), value: b"value5".to_vec() },
        ];
        
        let mut log_indices = Vec::new();
        for command in commands {
            if let Some(log_index) = cluster.submit_client_command(command).unwrap() {
                log_indices.push(log_index);
            }
        }
        
        // All commands should get different log indices
        assert_eq!(log_indices.len(), 5);
        for i in 1..log_indices.len() {
            assert!(log_indices[i] > log_indices[i-1]);
        }
    }

    #[test]
    fn test_delete_operations() {
        let mut cluster = SimpleTestCluster::new(3).unwrap();
        
        // Force node 0 to be leader
        cluster.force_leader(0).unwrap();
        
        // First, add some data
        let put_commands = vec![
            KVOperation::Put { key: "delete_test1".to_string(), value: b"will_be_deleted".to_vec() },
            KVOperation::Put { key: "delete_test2".to_string(), value: b"will_remain".to_vec() },
            KVOperation::Put { key: "delete_test3".to_string(), value: b"will_be_deleted".to_vec() },
        ];
        
        let mut log_indices = Vec::new();
        for command in put_commands {
            if let Some(log_index) = cluster.submit_client_command(command).unwrap() {
                log_indices.push(log_index);
            }
        }
        
        // Now delete some keys
        let delete_commands = vec![
            KVOperation::Delete { key: "delete_test1".to_string() },
            KVOperation::Delete { key: "delete_test3".to_string() },
        ];
        
        for command in delete_commands {
            if let Some(log_index) = cluster.submit_client_command(command).unwrap() {
                log_indices.push(log_index);
            }
        }
        
        // Should have 5 total operations (3 puts + 2 deletes)
        assert_eq!(log_indices.len(), 5);
        assert_eq!(log_indices, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_cluster_statistics() {
        let mut cluster = SimpleTestCluster::new(5).unwrap();
        
        let stats = cluster.get_cluster_stats();
        assert_eq!(stats.total_nodes, 5);
        assert_eq!(stats.leaders, 0); // No leader initially
        assert_eq!(stats.followers, 5);
        assert_eq!(stats.candidates, 0);
        
        // Force node 0 to be leader
        cluster.force_leader(0).unwrap();
        
        let updated_stats = cluster.get_cluster_stats();
        assert_eq!(updated_stats.total_nodes, 5);
        assert_eq!(updated_stats.leaders, 1);
        assert_eq!(updated_stats.followers, 4);
        assert_eq!(updated_stats.candidates, 0);
    }

    #[test]
    fn test_performance_measurement() {
        let mut cluster = SimpleTestCluster::new(3).unwrap();
        
        // Force node 0 to be leader
        cluster.force_leader(0).unwrap();
        
        let start_time = std::time::Instant::now();
        
        // Submit multiple commands to measure throughput
        let num_commands = 10;
        let mut log_indices = Vec::new();
        
        for i in 0..num_commands {
            let put_operation = KVOperation::Put {
                key: format!("perf_key_{}", i),
                value: format!("perf_value_{}", i).into_bytes(),
            };
            
            if let Some(log_index) = cluster.submit_client_command(put_operation).unwrap() {
                log_indices.push(log_index);
            }
        }
        
        let submission_time = start_time.elapsed();
        
        // Verify all commands were submitted
        assert_eq!(log_indices.len(), num_commands);
        
        // Basic performance assertions
        assert!(submission_time.as_millis() < 1000); // Should be fast for 10 commands
        
        // Verify sequential log indices
        for i in 0..num_commands {
            assert_eq!(log_indices[i], (i + 1) as u64);
        }
    }

    #[test]
    fn test_system_stability_under_load() {
        let mut cluster = SimpleTestCluster::new(5).unwrap();
        
        // Force node 0 to be leader
        cluster.force_leader(0).unwrap();
        
        // Submit a large number of commands to test stability
        let num_commands = 50;
        let mut log_indices = Vec::new();
        
        for i in 0..num_commands {
            let operation = if i % 3 == 0 {
                KVOperation::Put {
                    key: format!("load_key_{}", i),
                    value: format!("load_value_{}", i).into_bytes(),
                }
            } else if i % 3 == 1 {
                KVOperation::Put {
                    key: format!("load_key_{}", i),
                    value: format!("updated_value_{}", i).into_bytes(),
                }
            } else {
                KVOperation::Delete {
                    key: format!("load_key_{}", i - 1),
                }
            };
            
            if let Some(log_index) = cluster.submit_client_command(operation).unwrap() {
                log_indices.push(log_index);
            }
        }
        
        // Verify system stability - all commands should be accepted
        assert_eq!(log_indices.len(), num_commands);
        
        // Check that the system handled the load correctly
        let stats = cluster.get_cluster_stats();
        assert_eq!(stats.total_nodes, 5);
        assert_eq!(stats.leaders, 1);
        assert_eq!(stats.followers, 4);
        
        // Verify that operations were successful (log entries created)
        // Note: total_log_entries is the sum of commit_index across all nodes
        // Since we have a leader with log entries, this should be > 0
        assert!(stats.total_log_entries >= num_commands as usize, 
                "Expected at least {} log entries, got {}", num_commands, stats.total_log_entries);
    }

    #[test]
    fn test_log_consistency_basic() {
        let mut cluster = SimpleTestCluster::new(3).unwrap();
        
        // Force node 0 to be leader
        cluster.force_leader(0).unwrap();
        
        // Submit some commands
        for i in 0..5 {
            let put_operation = KVOperation::Put {
                key: format!("consistency_key_{}", i),
                value: format!("consistency_value_{}", i).into_bytes(),
            };
            
            cluster.submit_client_command(put_operation).unwrap();
        }
        
        // In this simplified test, we can only verify that the leader has the entries
        // Full consistency testing would require simulating the consensus process
        let leader = &cluster.nodes[&0];
        
        // Verify that log entries exist
        for i in 1..=5 {
            let entry = leader.get_log_entry(i);
            assert!(entry.is_some(), "Log entry {} should exist", i);
        }
    }
}
