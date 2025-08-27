//! kvapp-c: A Raft-based distributed key-value store
//! 
//! This application implements the Raft consensus algorithm to provide a distributed
//! key-value store that maintains consistency across multiple nodes.
//! 
//! # Architecture
//! 
//! The system follows an event-driven architecture with dependency injection:
//! - **raft**: Core Raft consensus algorithm implementation
//! - **storage**: Persistent storage abstractions and implementations
//! - **network**: Network communication and message bus
//! - **kv**: Key-value store operations and client interface
//! 
//! # Design Principles
//! 
//! - Synchronous design (no async runtime)
//! - Trait-based interfaces for dependency injection
//! - Event-driven communication between components
//! - Comprehensive testing with mock implementations

pub mod raft;
pub mod storage;
pub mod network;
pub mod kv;

#[cfg(test)]
pub mod tests;

// Re-export commonly used types
pub use raft::{RaftNode, RaftState};
pub use storage::{LogStorage, StateStorage};
pub use network::{NetworkTransport, MessageBus};
pub use kv::{KVStore, KVClient};

/// Common result type used throughout the application
pub type Result<T> = std::result::Result<T, Error>;

/// Main error type for the kvapp-c application
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// Storage-related errors
    Storage(String),
    /// Network-related errors
    Network(String),
    /// Raft consensus errors
    Raft(String),
    /// Key-value operation errors
    KeyValue(String),
    /// I/O errors
    Io(String),
    /// Serialization/deserialization errors
    Serialization(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Storage(msg) => write!(f, "Storage error: {}", msg),
            Error::Network(msg) => write!(f, "Network error: {}", msg),
            Error::Raft(msg) => write!(f, "Raft error: {}", msg),
            Error::KeyValue(msg) => write!(f, "Key-value error: {}", msg),
            Error::Io(msg) => write!(f, "I/O error: {}", msg),
            Error::Serialization(msg) => write!(f, "Serialization error: {}", msg),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err.to_string())
    }
}

/// Node identifier type
pub type NodeId = u64;

/// Raft term type
pub type Term = u64;

/// Log index type
pub type LogIndex = u64;

fn main() {
    // Initialize logging
    unsafe {
        std::env::set_var("RUST_LOG", "debug");
    }
    env_logger::init();
    
    log::info!("Starting kvapp-c: Raft-based distributed key-value store");
    
    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    
    if args.len() < 2 {
        print_usage(&args[0]);
        std::process::exit(1);
    }
    
    match args[1].as_str() {
        "server" => {
            if let Err(e) = run_server(&args[2..]) {
                log::error!("Server error: {}", e);
                std::process::exit(1);
            }
        }
        "client" => {
            if let Err(e) = run_client(&args[2..]) {
                log::error!("Client error: {}", e);
                std::process::exit(1);
            }
        }
        _ => {
            print_usage(&args[0]);
            std::process::exit(1);
        }
    }
}

fn print_usage(program_name: &str) {
    println!("Usage:");
    println!("  {} server --node-id <id> --bind <host:port> --cluster <node1:port1,node2:port2,...> [--data-dir <path>]", program_name);
    println!("  {} client --cluster <node1:port1,node2:port2,...>", program_name);
    println!();
    println!("Server mode:");
    println!("  --node-id <id>     Unique node identifier (0, 1, 2, ...)");
    println!("  --bind <host:port> Address to bind this server to");
    println!("  --cluster <nodes>  Comma-separated list of all cluster nodes");
    println!("  --data-dir <path>  Directory for persistent data (default: ./data)");
    println!();
    println!("Client mode:");
    println!("  --cluster <nodes>  Comma-separated list of cluster nodes to connect to");
    println!();
    println!("Examples:");
    println!("  {} server --node-id 0 --bind localhost:8080 --cluster localhost:8080,localhost:8081,localhost:8082", program_name);
    println!("  {} client --cluster localhost:8080,localhost:8081,localhost:8082", program_name);
}

fn run_server(args: &[String]) -> Result<()> {
    let config = parse_server_args(args)?;
    
    log::info!("Starting server with node ID {} on {}", config.node_id, config.bind_address);
    log::info!("Cluster nodes: {:?}", config.cluster_addresses);
    log::info!("Data directory: {}", config.data_dir);
    
    // Create data directory if it doesn't exist
    std::fs::create_dir_all(&config.data_dir)?;
    
    // Initialize storage backends
    let state_storage = Box::new(storage::FileStateStorage::new(
        format!("{}/state_{}.dat", config.data_dir, config.node_id).into()
    )?);
    
    let log_storage = Box::new(storage::FileLogStorage::new(
        format!("{}/log_{}.dat", config.data_dir, config.node_id).into()
    )?);
    
    // Initialize network transport
    let mut network_config = network::NetworkConfig::new(
        network::NodeAddress::new(config.node_id, "localhost".to_string(), 8080)
    );
    
    // Add all cluster addresses to network config
    for (node_id, address) in &config.cluster_addresses {
        // Parse host and port from address string
        let parts: Vec<&str> = address.split(':').collect();
        if parts.len() == 2 {
            let host = parts[0].to_string();
            if let Ok(port) = parts[1].parse::<u16>() {
                let node_address = network::NodeAddress::new(*node_id, host, port);
                network_config.add_node(node_address);
            }
        }
    }
    
    let mut transport = network::TcpTransport::new(
        network_config, 
        config.node_id, 
        &config.bind_address
    )?;
    
    // Start the TCP listener for incoming connections
    transport.start_listener()?;
    
    // Create Raft configuration
    let raft_config = raft::RaftConfig {
        node_id: config.node_id,
        cluster_nodes: config.cluster_addresses.keys().cloned().collect(),
        election_timeout_ms: (150, 300),
        heartbeat_interval_ms: 50,
    };
    
    // Initialize Raft node
    let mut raft_node = raft::RaftNode::with_dependencies(
        raft_config,
        state_storage,
        log_storage,
        Box::new(transport),
    )?;
    
    // Initialize KV store
    let kv_storage = Box::new(storage::FileKVStorage::new(
        format!("{}/kv_{}.dat", config.data_dir, config.node_id).into()
    )?);
    let mut kv_store = kv::InMemoryKVStore::with_storage(kv_storage);
    
    // Initialize message bus
    let message_bus = network::MessageBus::new();
    
    // Start TCP listener for incoming connections
    // Note: We need to start the listener before creating the RaftNode
    // For now, we'll handle this in the transport creation
    
    log::info!("Server initialized successfully");
    
    // Start the server event loop
    run_server_loop(&mut raft_node, &mut kv_store, &message_bus)?;
    
    Ok(())
}

fn run_client(args: &[String]) -> Result<()> {
    let config = parse_client_args(args)?;
    
    log::info!("Starting client, connecting to cluster: {:?}", config.cluster_addresses);
    
    // Initialize KV client (using first node for now)
    let client = kv::KVClient::new(0);
    
    println!("Connected to kvapp-c cluster");
    println!("Type 'help' for available commands, 'quit' to exit");
    
    // Start interactive client loop
    run_client_loop(client)?;
    
    Ok(())
}

fn run_server_loop(
    raft_node: &mut raft::RaftNode,
    kv_store: &mut kv::InMemoryKVStore,
    message_bus: &network::MessageBus,
) -> Result<()> {
    use std::time::{Duration, Instant};
    use std::thread;
    
    log::info!("Starting server event loop");
    
    let mut last_heartbeat = Instant::now();
    let heartbeat_interval = Duration::from_millis(50);
    
    loop {
        // Check for election timeout
        if raft_node.is_election_timeout_expired() {
            log::debug!("Election timeout expired");
            raft_node.handle_election_timeout()?;
        }
        
        // Send heartbeats if we're the leader
        if raft_node.is_leader() && last_heartbeat.elapsed() >= heartbeat_interval {
            raft_node.handle_heartbeat_timeout()?;
            last_heartbeat = Instant::now();
        }
        
        // Process any pending network messages
        if let Err(e) = process_network_messages(raft_node, kv_store) {
            log::error!("Error processing network messages: {}", e);
        }
        
        // Process any client requests
        // TODO: Implement client request processing
        
        // Small sleep to prevent busy waiting
        thread::sleep(Duration::from_millis(10));
        
        // Check for shutdown signal
        // TODO: Implement graceful shutdown
    }
}

fn run_client_loop(mut client: kv::KVClient) -> Result<()> {
    use std::io::{self, Write};
    
    loop {
        print!("kvapp> ");
        io::stdout().flush().unwrap();
        
        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(_) => {
                let input = input.trim();
                if input.is_empty() {
                    continue;
                }
                
                match input {
                    "quit" | "exit" => {
                        println!("Goodbye!");
                        break;
                    }
                    "help" => {
                        print_client_help();
                    }
                    _ => {
                        if let Err(e) = process_client_command(&mut client, input) {
                            println!("Error: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                println!("Error reading input: {}", e);
                break;
            }
        }
    }
    
    Ok(())
}

fn print_client_help() {
    println!("Available commands:");
    println!("  get <key>           Get value for key");
    println!("  put <key> <value>   Set key to value");
    println!("  delete <key>        Delete key");
    println!("  list                List all keys");
    println!("  help                Show this help");
    println!("  quit                Exit client");
}

fn process_client_command(client: &mut kv::KVClient, input: &str) -> Result<()> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.is_empty() {
        return Ok(());
    }
    
    match parts[0] {
        "get" => {
            if parts.len() != 2 {
                println!("Usage: get <key>");
                return Ok(());
            }
            
            let key = parts[1];
            match client.get(key.to_string())? {
                Some(value) => {
                    match String::from_utf8(value.clone()) {
                        Ok(s) => println!("{}", s),
                        Err(_) => println!("{:?}", value),
                    }
                }
                None => println!("Key not found"),
            }
        }
        "put" => {
            if parts.len() < 3 {
                println!("Usage: put <key> <value>");
                return Ok(());
            }
            
            let key = parts[1];
            let value = parts[2..].join(" ");
            client.put(key.to_string(), value.into_bytes())?;
            println!("OK");
        }
        "delete" => {
            if parts.len() != 2 {
                println!("Usage: delete <key>");
                return Ok(());
            }
            
            let key = parts[1];
            if client.delete(key.to_string())? {
                println!("OK");
            } else {
                println!("Key not found");
            }
        }
        "list" => {
            let keys = client.list_keys()?;
            if keys.is_empty() {
                println!("No keys found");
            } else {
                for key in keys {
                    println!("{}", key);
                }
            }
        }
        _ => {
            println!("Unknown command: {}. Type 'help' for available commands.", parts[0]);
        }
    }
    
    Ok(())
}

/// Process incoming network messages and forward them to the Raft node
fn process_network_messages(
    raft_node: &mut raft::RaftNode,
    _kv_store: &mut kv::InMemoryKVStore,
) -> Result<()> {
    // Get the transport from the Raft node to check for incoming messages
    let raw_messages = raft_node.get_transport().receive_messages();
    
    if !raw_messages.is_empty() {
        log::debug!("Processing {} incoming network messages", raw_messages.len());
    }
    
    // Process each incoming message
    for (from_node_id, message_bytes) in raw_messages {
        log::debug!("Processing message from node {} ({} bytes)", from_node_id, message_bytes.len());
        
        // Deserialize the message
        match raft::RaftMessage::from_bytes(&message_bytes) {
            Ok(raft_message) => {
                log::debug!("Deserialized Raft message: {:?}", raft_message);
                
                // Forward the message to the Raft node for processing
                match raft_message {
                    raft::RaftMessage::RequestVote(request) => {
                        match raft_node.handle_vote_request(request) {
                            Ok(response) => {
                                // Send response back to the requesting node
                                let response_message = raft::RaftMessage::RequestVoteResponse(response);
                                let response_bytes = response_message.to_bytes();
                                if let Err(e) = raft_node.get_transport().send_message(from_node_id, response_bytes) {
                                    log::error!("Error sending RequestVoteResponse to node {}: {}", from_node_id, e);
                                }
                            }
                            Err(e) => {
                                log::error!("Error handling RequestVote from node {}: {}", from_node_id, e);
                            }
                        }
                    }
                    raft::RaftMessage::RequestVoteResponse(response) => {
                        if let Err(e) = raft_node.handle_vote_response(response) {
                            log::error!("Error handling RequestVoteResponse from node {}: {}", from_node_id, e);
                        }
                    }
                    raft::RaftMessage::AppendEntries(request) => {
                        match raft_node.handle_append_entries(request) {
                            Ok(response) => {
                                // Send response back to the requesting node
                                let response_message = raft::RaftMessage::AppendEntriesResponse(response);
                                let response_bytes = response_message.to_bytes();
                                if let Err(e) = raft_node.get_transport().send_message(from_node_id, response_bytes) {
                                    log::error!("Error sending AppendEntriesResponse to node {}: {}", from_node_id, e);
                                }
                            }
                            Err(e) => {
                                log::error!("Error handling AppendEntries from node {}: {}", from_node_id, e);
                            }
                        }
                    }
                    raft::RaftMessage::AppendEntriesResponse(response) => {
                        if let Err(e) = raft_node.handle_append_entries_response(response, from_node_id) {
                            log::error!("Error handling AppendEntriesResponse from node {}: {}", from_node_id, e);
                        }
                    }
                }
            }
            Err(e) => {
                log::error!("Failed to deserialize message from node {}: {}", from_node_id, e);
            }
        }
    }
    
    Ok(())
}

#[derive(Debug)]
struct ServerConfig {
    node_id: NodeId,
    bind_address: String,
    cluster_addresses: std::collections::HashMap<NodeId, String>,
    data_dir: String,
}

#[derive(Debug)]
struct ClientConfig {
    cluster_addresses: Vec<String>,
}

fn parse_server_args(args: &[String]) -> Result<ServerConfig> {
    let mut node_id = None;
    let mut bind_address = None;
    let mut cluster_spec = None;
    let mut data_dir = "./data".to_string();
    
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--node-id" => {
                if i + 1 >= args.len() {
                    return Err(Error::Raft("Missing value for --node-id".to_string()));
                }
                node_id = Some(args[i + 1].parse::<NodeId>()
                    .map_err(|_| Error::Raft("Invalid node ID".to_string()))?);
                i += 2;
            }
            "--bind" => {
                if i + 1 >= args.len() {
                    return Err(Error::Raft("Missing value for --bind".to_string()));
                }
                bind_address = Some(args[i + 1].clone());
                i += 2;
            }
            "--cluster" => {
                if i + 1 >= args.len() {
                    return Err(Error::Raft("Missing value for --cluster".to_string()));
                }
                cluster_spec = Some(args[i + 1].clone());
                i += 2;
            }
            "--data-dir" => {
                if i + 1 >= args.len() {
                    return Err(Error::Raft("Missing value for --data-dir".to_string()));
                }
                data_dir = args[i + 1].clone();
                i += 2;
            }
            _ => {
                return Err(Error::Raft(format!("Unknown argument: {}", args[i])));
            }
        }
    }
    
    let node_id = node_id.ok_or_else(|| Error::Raft("Missing --node-id".to_string()))?;
    let bind_address = bind_address.ok_or_else(|| Error::Raft("Missing --bind".to_string()))?;
    let cluster_spec = cluster_spec.ok_or_else(|| Error::Raft("Missing --cluster".to_string()))?;
    
    let cluster_addresses = parse_cluster_spec(&cluster_spec)?;
    
    Ok(ServerConfig {
        node_id,
        bind_address,
        cluster_addresses,
        data_dir,
    })
}

fn parse_client_args(args: &[String]) -> Result<ClientConfig> {
    let mut cluster_spec = None;
    
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--cluster" => {
                if i + 1 >= args.len() {
                    return Err(Error::Raft("Missing value for --cluster".to_string()));
                }
                cluster_spec = Some(args[i + 1].clone());
                i += 2;
            }
            _ => {
                return Err(Error::Raft(format!("Unknown argument: {}", args[i])));
            }
        }
    }
    
    let cluster_spec = cluster_spec.ok_or_else(|| Error::Raft("Missing --cluster".to_string()))?;
    let cluster_addresses: Vec<String> = cluster_spec.split(',').map(|s| s.trim().to_string()).collect();
    
    Ok(ClientConfig {
        cluster_addresses,
    })
}

fn parse_cluster_spec(cluster_spec: &str) -> Result<std::collections::HashMap<NodeId, String>> {
    let mut cluster_addresses = std::collections::HashMap::new();
    
    for (index, address) in cluster_spec.split(',').enumerate() {
        let address = address.trim().to_string();
        cluster_addresses.insert(index as NodeId, address);
    }
    
    Ok(cluster_addresses)
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let storage_err = Error::Storage("test error".to_string());
        assert_eq!(storage_err.to_string(), "Storage error: test error");
        
        let network_err = Error::Network("connection failed".to_string());
        assert_eq!(network_err.to_string(), "Network error: connection failed");
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let kvapp_err: Error = io_err.into();
        
        match kvapp_err {
            Error::Io(msg) => assert!(msg.contains("file not found")),
            _ => panic!("Expected Io error"),
        }
    }
}
