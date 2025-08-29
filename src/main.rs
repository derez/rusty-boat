//! kvapp-c: A Raft-based distributed key-value store application
//! 
//! This binary provides a command-line interface for running distributed
//! key-value store servers and clients using the kvapp-c library.

use kvapp_c::{
    self as kvapp,
    storage, network, kv, raft,
    NodeId, Result, Error,
};

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
    println!("  {} server --node-id <id> --bind <host:port> --cluster <node1:port1,node2:port2,...> [--data-dir <path>] [--verbose]", program_name);
    println!("  {} client --cluster <node1:port1,node2:port2,...> [--verbose]", program_name);
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
    println!("Global options:");
    println!("  --verbose, -v      Enable debug logging");
    println!();
    println!("Examples:");
    println!("  {} server --node-id 0 --bind localhost:8080 --cluster localhost:8080,localhost:8081,localhost:8082", program_name);
    println!("  {} client --cluster localhost:8080,localhost:8081,localhost:8082 --verbose", program_name);
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
            if let Ok(port) = parts[1].parse::<u16>() {
                let node_address = network::NodeAddress::new(*node_id, host, port);
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
        if let Err(e) = process_client_requests(raft_node, kv_store) {
            log::error!("Error processing client requests: {}", e);
        }
        
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

/// Process client requests and apply them to the Raft log
fn process_client_requests(
    raft_node: &mut raft::RaftNode,
    kv_store: &mut kv::InMemoryKVStore,
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
            
            // Use the proper KV store integration method
            if let Err(e) = network::TcpTransport::process_client_request_with_store(
                &request_data, 
                &mut stream, 
                kv_store, 
                raft_node.node_id()
            ) {
                log::error!("Error processing client request: {}", e);
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

fn parse_cluster_spec(cluster_spec: &str) -> Result<std::collections::HashMap<NodeId, String>> {
    let mut cluster_addresses = std::collections::HashMap::new();
    
    for (index, address) in cluster_spec.split(',').enumerate() {
        let address = address.trim().to_string();
        cluster_addresses.insert(index as NodeId, address);
    }
    
    Ok(cluster_addresses)
}
