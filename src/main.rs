//! kvapp-c: A Raft-based distributed key-value store application
//! 
//! This binary provides a command-line interface for running distributed
//! key-value store servers and clients using the kvapp-c library.

use kvapp_c::{
    self as kvapp,
    storage, network, kv, raft, timing,
    NodeId, Result, Error, TimingConfig, TimingMode,
};
use kvapp_c::kv::KVStore;

fn main() {
    // Parse command line arguments first to check for --verbose
    let args: Vec<String> = std::env::args().collect();
    
    // Check for --verbose flag in any position
    let verbose = args.iter().any(|arg| arg == "--verbose" || arg == "-v");
    
    // Initialize logging
    if std::env::var("RUST_LOG").is_err() {
        let log_level = if verbose {
            "kvapp_c=debug"
        } else {
            "kvapp_c=warn"
        };
        
        unsafe {
            std::env::set_var("RUST_LOG", log_level);
        }
    }
    env_logger::init();
    
    if verbose {
        log::debug!("Verbose logging enabled");
    }
    
    log::info!("Starting kvapp-c: Raft-based distributed key-value store");
    
    if args.len() < 2 {
        print_usage(&args[0]);
        std::process::exit(1);
    }
    
    match args[1].as_str() {
        "server" => {
            if let Err(e) = run_server(&args[2..], verbose) {
                log::error!("Server error: {}", e);
                std::process::exit(1);
            }
        }
        "client" => {
            if let Err(e) = run_client(&args[2..], verbose) {
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
    println!("  {} server --node-id <id> --bind <host:port> --cluster <node1:port1,node2:port2,...> [OPTIONS]", program_name);
    println!("  {} client --cluster <node1:port1,node2:port2,...> [OPTIONS]", program_name);
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
    println!("Timing options (server mode only):");
    println!("  --timing-mode <mode>        Preset timing mode: fast, debug, demo");
    println!("  --fast-mode                 Use fast timing (production, default)");
    println!("  --debug-mode                Use debug timing (moderate delays)");
    println!("  --demo-mode                 Use demo timing (slow for observation)");
    println!("  --event-loop-delay <ms>     Event loop delay in milliseconds");
    println!("  --heartbeat-interval <ms>   Heartbeat interval in milliseconds");
    println!("  --election-timeout-min <ms> Minimum election timeout in milliseconds");
    println!("  --election-timeout-max <ms> Maximum election timeout in milliseconds");
    println!("  --network-delay <ms>        Network message delay in milliseconds");
    println!("  --client-delay <ms>         Client request delay in milliseconds");
    println!();
    println!("Global options:");
    println!("  --verbose, -v      Enable debug logging");
    println!();
    println!("Examples:");
    println!("  # Start server with default (fast) timing");
    println!("  {} server --node-id 0 --bind localhost:8080 --cluster localhost:8080,localhost:8081,localhost:8082", program_name);
    println!();
    println!("  # Start server with debug timing for easier log reading");
    println!("  {} server --node-id 0 --bind localhost:8080 --cluster localhost:8080,localhost:8081,localhost:8082 --debug-mode --verbose", program_name);
    println!();
    println!("  # Start server with demo timing for slow observation");
    println!("  {} server --node-id 0 --bind localhost:8080 --cluster localhost:8080,localhost:8081,localhost:8082 --demo-mode --verbose", program_name);
    println!();
    println!("  # Start server with custom timing");
    println!("  {} server --node-id 0 --bind localhost:8080 --cluster localhost:8080,localhost:8081,localhost:8082 --event-loop-delay 500 --heartbeat-interval 1000", program_name);
    println!();
    println!("  # Start interactive client (automatically connects to client ports +1000)");
    println!("  {} client --cluster localhost:8080,localhost:8081,localhost:8082 --verbose", program_name);
    println!();
    println!("Network Architecture:");
    println!("  Servers listen on dual ports:");
    println!("    - Raft port (specified): Inter-node consensus communication");
    println!("    - Client port (+1000): Client request handling");
    println!("  Example: Server on 8080 → Raft: 8080, Client: 9080");
    println!("  Clients automatically connect to client ports (+1000 offset)");
    println!();
    println!("Timing modes:");
    println!("  fast:  Production timing (10ms event loop, 50ms heartbeat, 150-300ms election)");
    println!("  debug: Debug timing (100ms event loop, 500ms heartbeat, 1500-3000ms election)");
    println!("  demo:  Demo timing (1000ms event loop, 2000ms heartbeat, 5000-10000ms election)");
}

fn run_server(args: &[String], verbose: bool) -> Result<()> {
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
            if let Ok(raft_port) = parts[1].parse::<u16>() {
                let node_address = network::NodeAddress::new(*node_id, host, raft_port);
                network_config.add_node(node_address);
            }
        }
    }
    
    let transport = network::TcpTransport::new(
        network_config, 
        config.node_id, 
        &config.bind_address
    )?;
    
    // Start the TCP listener for incoming connections
    transport.start_listener()?;
    
    // Create Raft configuration with timing from parsed arguments
    let raft_config = raft::RaftConfig::new(
        config.node_id,
        config.cluster_addresses.keys().cloned().collect(),
        config.timing.clone(),
    );
    
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
    run_server_loop(&mut raft_node, &mut kv_store, &message_bus, &config.timing)?;
    
    Ok(())
}

fn run_client(args: &[String], verbose: bool) -> Result<()> {
    let config = parse_client_args(args)?;
    
    log::info!("Starting client, connecting to cluster: {:?}", config.cluster_addresses);
    
    // Initialize KV client with cluster addresses
    let client = kv::KVClient::with_cluster_addresses(0, config.cluster_addresses.clone());
    
    log::info!("Connected to kvapp-c cluster");
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
    timing_config: &TimingConfig,
) -> Result<()> {
    use std::time::{Duration, Instant};
    use std::thread;
    
    log::info!("Starting server event loop with timing config: event_loop_delay={}ms, heartbeat_interval={}ms", 
               timing_config.event_loop_delay_ms, timing_config.heartbeat_interval_ms);
    
    let mut last_heartbeat = Instant::now();
    let heartbeat_interval = timing_config.heartbeat_interval();
    let mut last_applied_to_kv = 0u64; // Track what we've applied to KV store
    
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
        if let Err(e) = process_network_messages(raft_node, kv_store, timing_config) {
            log::error!("Error processing network messages: {}", e);
        }
        
        // Process any client requests
        if let Err(e) = process_client_requests(raft_node, kv_store, timing_config) {
            log::error!("Error processing client requests: {}", e);
        }
        
        // Apply committed entries to KV store
        if let Err(e) = apply_committed_entries_to_kv_store(raft_node, kv_store, &mut last_applied_to_kv) {
            log::error!("Error applying committed entries to KV store: {}", e);
        }
        
        // Apply configurable event loop delay
        timing_config.apply_event_loop_delay();
        
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
                        log::info!("Client session ending");
                        println!("Goodbye!");
                        break;
                    }
                    "help" => {
                        print_client_help();
                    }
                    _ => {
                        if let Err(e) = process_client_command(&mut client, input) {
                            log::error!("Client command error: {}", e);
                            println!("Error: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                log::error!("Error reading input: {}", e);
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
            log::debug!("Client executing GET command for key: {}", key);
            match client.get(key.to_string())? {
                Some(value) => {
                    log::debug!("GET command successful for key: {}", key);
                    match String::from_utf8(value.clone()) {
                        Ok(s) => println!("{}", s),
                        Err(_) => println!("{:?}", value),
                    }
                }
                None => {
                    log::debug!("GET command: key not found: {}", key);
                    println!("Key not found");
                }
            }
        }
        "put" => {
            if parts.len() < 3 {
                println!("Usage: put <key> <value>");
                return Ok(());
            }
            
            let key = parts[1];
            let value = parts[2..].join(" ");
            log::debug!("Client executing PUT command for key: {}", key);
            client.put(key.to_string(), value.into_bytes())?;
            log::debug!("PUT command successful for key: {}", key);
            println!("OK");
        }
        "delete" => {
            if parts.len() != 2 {
                println!("Usage: delete <key>");
                return Ok(());
            }
            
            let key = parts[1];
            log::debug!("Client executing DELETE command for key: {}", key);
            if client.delete(key.to_string())? {
                log::debug!("DELETE command successful for key: {}", key);
                println!("OK");
            } else {
                log::debug!("DELETE command: key not found: {}", key);
                println!("Key not found");
            }
        }
        "list" => {
            log::debug!("Client executing LIST command");
            let keys = client.list_keys()?;
            log::debug!("LIST command returned {} keys", keys.len());
            if keys.is_empty() {
                println!("No keys found");
            } else {
                for key in keys {
                    println!("{}", key);
                }
            }
        }
        _ => {
            log::debug!("Client received unknown command: {}", parts[0]);
            println!("Unknown command: {}. Type 'help' for available commands.", parts[0]);
        }
    }
    
    Ok(())
}

/// Process incoming network messages and forward them to the Raft node
fn process_network_messages(
    raft_node: &mut raft::RaftNode,
    _kv_store: &mut kv::InMemoryKVStore,
    timing_config: &TimingConfig,
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

/// Process client requests through the Raft log (Raft-compliant)
fn process_client_requests(
    raft_node: &mut raft::RaftNode,
    kv_store: &mut kv::InMemoryKVStore,
    timing_config: &TimingConfig,
) -> Result<()> {
    // Get the transport and check for client requests
    let transport = raft_node.get_transport();
    
    // Try to downcast to TcpTransport to access client request methods
    if let Some(tcp_transport) = transport.as_any().downcast_ref::<network::TcpTransport>() {
        // Get any pending client requests
        let client_requests = tcp_transport.get_client_requests();
        
        if !client_requests.is_empty() {
            log::debug!("Processing {} client requests", client_requests.len());
        }
        
        // Process each client request
        for (mut stream, request_data) in client_requests {
            log::debug!("Processing client request ({} bytes)", request_data.len());
            
            if let Err(e) = process_single_client_request(raft_node, &request_data, &mut stream) {
                log::error!("Error processing client request: {}", e);
            }
        }
    }
    
    Ok(())
}

/// Process a single client request through Raft consensus
fn process_single_client_request(
    raft_node: &mut raft::RaftNode,
    request_data: &[u8],
    stream: &mut std::net::TcpStream,
) -> Result<()> {
    use std::io::Write;
    use kvapp_c::kv::{KVOperation, KVResponse};
    
    // Deserialize the client request
    let operation = match KVOperation::from_bytes(request_data) {
        Ok(op) => op,
        Err(e) => {
            log::warn!("Failed to deserialize client request: {}", e);
            send_error_response(stream, "Invalid request format")?;
            return Ok(());
        }
    };
    
    log::debug!("Client request: {:?}", operation);
    
    // Handle read operations directly (no need for Raft consensus)
    if matches!(operation, KVOperation::Get { .. } | KVOperation::List) {
        // For now, return an error since we need to integrate with the KV store
        // TODO: Implement direct read operations from committed state
        send_error_response(stream, "Read operations not yet implemented with Raft integration")?;
        return Ok(());
    }
    
    // Handle write operations through Raft consensus
    match operation {
        KVOperation::Put { .. } | KVOperation::Delete { .. } => {
            // Check if this node is the leader
            if !raft_node.is_leader() {
                // Redirect to leader if known
                if let Some(leader_id) = raft_node.get_current_leader() {
                    log::info!("Redirecting client request to leader {}", leader_id);
                    send_error_response(stream, &format!("Not leader. Current leader: {}", leader_id))?;
                } else {
                    log::info!("No leader known, rejecting client request");
                    send_error_response(stream, "No leader available. Try again later.")?;
                }
                return Ok(());
            }
            
            // Serialize the operation for the Raft log
            let command_data = operation.to_bytes();
            
            // Append the command to the Raft log
            match raft_node.append_client_command(command_data) {
                Ok(log_index) => {
                    log::info!("Client command appended to Raft log at index {}", log_index);
                    
                    // For now, send a simple success response
                    // TODO: Implement proper async response after log commitment
                    let response = match operation {
                        KVOperation::Put { key, .. } => KVResponse::Put { key },
                        KVOperation::Delete { key } => KVResponse::Delete { key },
                        _ => unreachable!(),
                    };
                    
                    send_success_response(stream, &response)?;
                }
                Err(e) => {
                    log::error!("Failed to append client command to Raft log: {}", e);
                    send_error_response(stream, "Failed to process command")?;
                }
            }
        }
        KVOperation::Get { .. } | KVOperation::List => {
            // Already handled above
            unreachable!();
        }
    }
    
    Ok(())
}

/// Send a success response to the client
fn send_success_response(stream: &mut std::net::TcpStream, response: &kvapp_c::kv::KVResponse) -> Result<()> {
    use std::io::Write;
    
    // Serialize the response (simplified for now)
    let response_data = match response {
        kvapp_c::kv::KVResponse::Put { key } => {
            let mut data = vec![1u8]; // Put response type
            let key_len = key.len() as u32;
            data.extend_from_slice(&key_len.to_be_bytes());
            data.extend_from_slice(key.as_bytes());
            data
        }
        kvapp_c::kv::KVResponse::Delete { key } => {
            let mut data = vec![2u8]; // Delete response type
            let key_len = key.len() as u32;
            data.extend_from_slice(&key_len.to_be_bytes());
            data.extend_from_slice(key.as_bytes());
            data
        }
        _ => {
            return Err(Error::Serialization("Unsupported response type".to_string()));
        }
    };
    
    // Send framed response
    let response_len = response_data.len() as u32;
    let mut framed_response = Vec::with_capacity(4 + response_data.len());
    framed_response.extend_from_slice(&response_len.to_be_bytes());
    framed_response.extend_from_slice(&response_data);
    
    stream.write_all(&framed_response)?;
    stream.flush()?;
    
    log::debug!("Sent success response ({} bytes)", framed_response.len());
    Ok(())
}

/// Send an error response to the client
fn send_error_response(stream: &mut std::net::TcpStream, message: &str) -> Result<()> {
    use std::io::Write;
    
    // Create error response
    let mut response_data = vec![3u8]; // Error response type
    response_data.extend_from_slice(message.as_bytes());
    
    // Send framed response
    let response_len = response_data.len() as u32;
    let mut framed_response = Vec::with_capacity(4 + response_data.len());
    framed_response.extend_from_slice(&response_len.to_be_bytes());
    framed_response.extend_from_slice(&response_data);
    
    stream.write_all(&framed_response)?;
    stream.flush()?;
    
    log::debug!("Sent error response: {} ({} bytes)", message, framed_response.len());
    Ok(())
}


#[derive(Debug)]
struct ServerConfig {
    node_id: NodeId,
    bind_address: String,
    cluster_addresses: std::collections::HashMap<NodeId, String>,
    data_dir: String,
    timing: TimingConfig,
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
    
    // Timing configuration parameters
    let mut timing_mode: Option<TimingMode> = None;
    let mut event_loop_delay: Option<u64> = None;
    let mut heartbeat_interval: Option<u64> = None;
    let mut election_timeout_min: Option<u64> = None;
    let mut election_timeout_max: Option<u64> = None;
    let mut network_delay: Option<u64> = None;
    let mut client_delay: Option<u64> = None;
    
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
            "--timing-mode" => {
                if i + 1 >= args.len() {
                    return Err(Error::Raft("Missing value for --timing-mode".to_string()));
                }
                timing_mode = Some(args[i + 1].parse::<TimingMode>()
                    .map_err(|e| Error::Raft(format!("Invalid timing mode: {}", e)))?);
                i += 2;
            }
            "--event-loop-delay" => {
                if i + 1 >= args.len() {
                    return Err(Error::Raft("Missing value for --event-loop-delay".to_string()));
                }
                event_loop_delay = Some(args[i + 1].parse::<u64>()
                    .map_err(|_| Error::Raft("Invalid event loop delay".to_string()))?);
                i += 2;
            }
            "--heartbeat-interval" => {
                if i + 1 >= args.len() {
                    return Err(Error::Raft("Missing value for --heartbeat-interval".to_string()));
                }
                heartbeat_interval = Some(args[i + 1].parse::<u64>()
                    .map_err(|_| Error::Raft("Invalid heartbeat interval".to_string()))?);
                i += 2;
            }
            "--election-timeout-min" => {
                if i + 1 >= args.len() {
                    return Err(Error::Raft("Missing value for --election-timeout-min".to_string()));
                }
                election_timeout_min = Some(args[i + 1].parse::<u64>()
                    .map_err(|_| Error::Raft("Invalid election timeout min".to_string()))?);
                i += 2;
            }
            "--election-timeout-max" => {
                if i + 1 >= args.len() {
                    return Err(Error::Raft("Missing value for --election-timeout-max".to_string()));
                }
                election_timeout_max = Some(args[i + 1].parse::<u64>()
                    .map_err(|_| Error::Raft("Invalid election timeout max".to_string()))?);
                i += 2;
            }
            "--network-delay" => {
                if i + 1 >= args.len() {
                    return Err(Error::Raft("Missing value for --network-delay".to_string()));
                }
                network_delay = Some(args[i + 1].parse::<u64>()
                    .map_err(|_| Error::Raft("Invalid network delay".to_string()))?);
                i += 2;
            }
            "--client-delay" => {
                if i + 1 >= args.len() {
                    return Err(Error::Raft("Missing value for --client-delay".to_string()));
                }
                client_delay = Some(args[i + 1].parse::<u64>()
                    .map_err(|_| Error::Raft("Invalid client delay".to_string()))?);
                i += 2;
            }
            "--fast-mode" => {
                timing_mode = Some(TimingMode::Fast);
                i += 1;
            }
            "--debug-mode" => {
                timing_mode = Some(TimingMode::Debug);
                i += 1;
            }
            "--demo-mode" => {
                timing_mode = Some(TimingMode::Demo);
                i += 1;
            }
            "--verbose" | "-v" => {
                // Skip verbose flag, already handled in main
                i += 1;
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
    
    // Build timing configuration
    let timing = if let Some(mode) = timing_mode {
        let mut config = mode.to_config();
        
        // Override with custom values if provided
        if let Some(delay) = event_loop_delay {
            config.event_loop_delay_ms = delay;
        }
        if let Some(interval) = heartbeat_interval {
            config.heartbeat_interval_ms = interval;
        }
        if let Some(min) = election_timeout_min {
            config.election_timeout_min_ms = min;
        }
        if let Some(max) = election_timeout_max {
            config.election_timeout_max_ms = max;
        }
        if let Some(delay) = network_delay {
            config.network_delay_ms = delay;
        }
        if let Some(delay) = client_delay {
            config.client_delay_ms = delay;
        }
        
        // Validate the final configuration
        if config.election_timeout_min_ms >= config.election_timeout_max_ms {
            return Err(Error::Raft("Election timeout minimum must be less than maximum".to_string()));
        }
        if config.heartbeat_interval_ms >= config.election_timeout_min_ms {
            return Err(Error::Raft("Heartbeat interval must be less than election timeout minimum".to_string()));
        }
        
        config
    } else if event_loop_delay.is_some() || heartbeat_interval.is_some() || 
              election_timeout_min.is_some() || election_timeout_max.is_some() ||
              network_delay.is_some() || client_delay.is_some() {
        // Custom timing parameters provided without mode
        TimingConfig::custom(
            event_loop_delay.unwrap_or(10),
            heartbeat_interval.unwrap_or(50),
            election_timeout_min.unwrap_or(150),
            election_timeout_max.unwrap_or(300),
            network_delay.unwrap_or(0),
            client_delay.unwrap_or(0),
        )?
    } else {
        // Default to fast mode
        TimingConfig::fast()
    };
    
    Ok(ServerConfig {
        node_id,
        bind_address,
        cluster_addresses,
        data_dir,
        timing,
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
            "--verbose" | "-v" => {
                // Skip verbose flag, already handled in main
                i += 1;
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

/// Apply committed entries from Raft log to KV store
fn apply_committed_entries_to_kv_store(
    raft_node: &mut raft::RaftNode,
    kv_store: &mut kv::InMemoryKVStore,
    last_applied_to_kv: &mut u64,
) -> Result<()> {
    let commit_index = raft_node.get_commit_index();
    
    // Apply any newly committed entries to the KV store
    while *last_applied_to_kv < commit_index {
        *last_applied_to_kv += 1;
        
        // Get the log entry at this index
        if let Some(log_entry) = raft_node.get_log_entry(*last_applied_to_kv) {
            log::debug!(
                "Applying committed log entry {} to KV store (term: {}, {} bytes)",
                *last_applied_to_kv, log_entry.term, log_entry.data.len()
            );
            
            // Apply the entry to the KV store
            match kv_store.apply_entry(&log_entry) {
                Ok(response) => {
                    log::info!(
                        "Successfully applied log entry {} to KV store: {:?}",
                        *last_applied_to_kv, response
                    );
                }
                Err(e) => {
                    log::error!(
                        "Failed to apply log entry {} to KV store: {}",
                        *last_applied_to_kv, e
                    );
                    // Continue applying other entries even if one fails
                }
            }
        } else {
            log::warn!(
                "Missing log entry {} when trying to apply to KV store",
                *last_applied_to_kv
            );
            break;
        }
    }
    
    Ok(())
}

fn parse_cluster_spec(cluster_spec: &str) -> Result<std::collections::HashMap<NodeId, String>> {
    let mut cluster_addresses = std::collections::HashMap::new();
    
    for (index, address) in cluster_spec.split(',').enumerate() {
        let address = address.trim().to_string();
        cluster_addresses.insert(index as NodeId, address);
    }
    
    Ok(cluster_addresses)
}
