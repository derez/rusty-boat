
# Objectives
* Create a Key Value application built on the Raft Consensus Algorithm. 
* Design this application to be fully testable and debuggable.

# Raft Consensus Algorithm
* Original paper at https://raft.github.io/raft.pdf
* Additional information can be found at https://raft.github.io

# Implementation 

## Language
* Rust Edition 2024

## Development Environment
- **Windows 10** - Primary development platform
- **VSCode** - IDE with Rust analyzer extension
- **Git** - Version control

## Design Decisions
* No async runtime
* Minimal dependencies
* Use Standard library when possible
* Well defined Interfaces
* Testability 

## Design Patterns
* Dependency Injection pattern should be used when possible to de-couple components

## Architecture
* Event-Driven Architecture
