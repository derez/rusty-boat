//! Timing configuration for controlling event loop and communication delays
//! 
//! This module provides configurable timing parameters to control the speed
//! of various operations in the distributed system, allowing for better
//! log readability and debugging.

use crate::Result;

/// Configuration for timing parameters throughout the system
#[derive(Debug, Clone, PartialEq)]
pub struct TimingConfig {
    /// Delay between main event loop iterations (milliseconds)
    pub event_loop_delay_ms: u64,
    
    /// Interval between heartbeat messages from leader (milliseconds)
    pub heartbeat_interval_ms: u64,
    
    /// Minimum election timeout (milliseconds)
    pub election_timeout_min_ms: u64,
    
    /// Maximum election timeout (milliseconds)
    pub election_timeout_max_ms: u64,
    
    /// Artificial delay for network message processing (milliseconds)
    pub network_delay_ms: u64,
    
    /// Artificial delay for client request processing (milliseconds)
    pub client_delay_ms: u64,
}

impl TimingConfig {
    /// Create a new timing configuration with default values
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Create timing configuration optimized for production performance
    pub fn fast() -> Self {
        Self {
            event_loop_delay_ms: 10,
            heartbeat_interval_ms: 50,
            election_timeout_min_ms: 150,
            election_timeout_max_ms: 300,
            network_delay_ms: 0,
            client_delay_ms: 0,
        }
    }
    
    /// Create timing configuration optimized for debugging
    pub fn debug() -> Self {
        Self {
            event_loop_delay_ms: 100,
            heartbeat_interval_ms: 500,
            election_timeout_min_ms: 1000,
            election_timeout_max_ms: 2000,
            network_delay_ms: 50,
            client_delay_ms: 25,
        }
    }
    
    /// Create timing configuration optimized for demonstrations and learning
    pub fn demo() -> Self {
        Self {
            event_loop_delay_ms: 1000,
            heartbeat_interval_ms: 3000,
            election_timeout_min_ms: 5000,
            election_timeout_max_ms: 8000,
            network_delay_ms: 500,
            client_delay_ms: 200,
        }
    }
    
    /// Create timing configuration from individual parameters
    pub fn custom(
        event_loop_delay_ms: u64,
        heartbeat_interval_ms: u64,
        election_timeout_min_ms: u64,
        election_timeout_max_ms: u64,
        network_delay_ms: u64,
        client_delay_ms: u64,
    ) -> Result<Self> {
        // Validate timing parameters
        if election_timeout_min_ms >= election_timeout_max_ms {
            return Err(crate::Error::Raft(
                "Election timeout minimum must be less than maximum".to_string()
            ));
        }
        
        if heartbeat_interval_ms >= election_timeout_min_ms {
            return Err(crate::Error::Raft(
                "Heartbeat interval must be less than election timeout minimum".to_string()
            ));
        }
        
        Ok(Self {
            event_loop_delay_ms,
            heartbeat_interval_ms,
            election_timeout_min_ms,
            election_timeout_max_ms,
            network_delay_ms,
            client_delay_ms,
        })
    }
    
    /// Get election timeout range as tuple (min, max)
    pub fn election_timeout_range(&self) -> (u64, u64) {
        (self.election_timeout_min_ms, self.election_timeout_max_ms)
    }
    
    /// Apply network delay if configured
    pub fn apply_network_delay(&self) {
        if self.network_delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(self.network_delay_ms));
        }
    }
    
    /// Apply client processing delay if configured
    pub fn apply_client_delay(&self) {
        if self.client_delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(self.client_delay_ms));
        }
    }
    
    /// Apply event loop delay
    pub fn apply_event_loop_delay(&self) {
        std::thread::sleep(std::time::Duration::from_millis(self.event_loop_delay_ms));
    }
    
    /// Get heartbeat interval as Duration
    pub fn heartbeat_interval(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.heartbeat_interval_ms)
    }
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self::fast()
    }
}

/// Timing mode presets for easy configuration
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimingMode {
    /// Fast mode for production performance
    Fast,
    /// Debug mode for development and debugging
    Debug,
    /// Demo mode for presentations and learning
    Demo,
    /// Custom mode with user-specified parameters
    Custom,
}

impl TimingMode {
    /// Convert timing mode to TimingConfig
    pub fn to_config(self) -> TimingConfig {
        match self {
            TimingMode::Fast => TimingConfig::fast(),
            TimingMode::Debug => TimingConfig::debug(),
            TimingMode::Demo => TimingConfig::demo(),
            TimingMode::Custom => TimingConfig::default(), // Will be overridden with custom values
        }
    }
}

impl std::str::FromStr for TimingMode {
    type Err = crate::Error;
    
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "fast" => Ok(TimingMode::Fast),
            "debug" => Ok(TimingMode::Debug),
            "demo" => Ok(TimingMode::Demo),
            "custom" => Ok(TimingMode::Custom),
            _ => Err(crate::Error::Raft(format!("Invalid timing mode: {}", s))),
        }
    }
}

impl std::fmt::Display for TimingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TimingMode::Fast => write!(f, "fast"),
            TimingMode::Debug => write!(f, "debug"),
            TimingMode::Demo => write!(f, "demo"),
            TimingMode::Custom => write!(f, "custom"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timing_config_creation() {
        let config = TimingConfig::new();
        assert_eq!(config, TimingConfig::default());
        assert_eq!(config, TimingConfig::fast());
    }

    #[test]
    fn test_timing_presets() {
        let fast = TimingConfig::fast();
        assert_eq!(fast.event_loop_delay_ms, 10);
        assert_eq!(fast.heartbeat_interval_ms, 50);
        assert_eq!(fast.network_delay_ms, 0);

        let debug = TimingConfig::debug();
        assert_eq!(debug.event_loop_delay_ms, 100);
        assert_eq!(debug.heartbeat_interval_ms, 500);
        assert_eq!(debug.network_delay_ms, 50);

        let demo = TimingConfig::demo();
        assert_eq!(demo.event_loop_delay_ms, 1000);
        assert_eq!(demo.heartbeat_interval_ms, 3000);
        assert_eq!(demo.network_delay_ms, 500);
    }

    #[test]
    fn test_custom_timing_config() {
        let config = TimingConfig::custom(20, 100, 500, 1000, 10, 5).unwrap();
        assert_eq!(config.event_loop_delay_ms, 20);
        assert_eq!(config.heartbeat_interval_ms, 100);
        assert_eq!(config.election_timeout_min_ms, 500);
        assert_eq!(config.election_timeout_max_ms, 1000);
        assert_eq!(config.network_delay_ms, 10);
        assert_eq!(config.client_delay_ms, 5);
    }

    #[test]
    fn test_custom_timing_validation() {
        // Election timeout min >= max should fail
        let result = TimingConfig::custom(10, 50, 1000, 500, 0, 0);
        assert!(result.is_err());

        // Heartbeat interval >= election timeout min should fail
        let result = TimingConfig::custom(10, 1000, 500, 1000, 0, 0);
        assert!(result.is_err());

        // Valid configuration should succeed
        let result = TimingConfig::custom(10, 100, 500, 1000, 0, 0);
        assert!(result.is_ok());
    }

    #[test]
    fn test_election_timeout_range() {
        let config = TimingConfig::debug();
        let (min, max) = config.election_timeout_range();
        assert_eq!(min, 1000);
        assert_eq!(max, 2000);
    }

    #[test]
    fn test_heartbeat_interval() {
        let config = TimingConfig::fast();
        let interval = config.heartbeat_interval();
        assert_eq!(interval, std::time::Duration::from_millis(50));
    }

    #[test]
    fn test_timing_mode_parsing() {
        assert_eq!("fast".parse::<TimingMode>().unwrap(), TimingMode::Fast);
        assert_eq!("debug".parse::<TimingMode>().unwrap(), TimingMode::Debug);
        assert_eq!("demo".parse::<TimingMode>().unwrap(), TimingMode::Demo);
        assert_eq!("custom".parse::<TimingMode>().unwrap(), TimingMode::Custom);
        
        // Case insensitive
        assert_eq!("FAST".parse::<TimingMode>().unwrap(), TimingMode::Fast);
        assert_eq!("Debug".parse::<TimingMode>().unwrap(), TimingMode::Debug);
        
        // Invalid mode should fail
        assert!("invalid".parse::<TimingMode>().is_err());
    }

    #[test]
    fn test_timing_mode_display() {
        assert_eq!(TimingMode::Fast.to_string(), "fast");
        assert_eq!(TimingMode::Debug.to_string(), "debug");
        assert_eq!(TimingMode::Demo.to_string(), "demo");
        assert_eq!(TimingMode::Custom.to_string(), "custom");
    }

    #[test]
    fn test_timing_mode_to_config() {
        let fast_config = TimingMode::Fast.to_config();
        assert_eq!(fast_config, TimingConfig::fast());

        let debug_config = TimingMode::Debug.to_config();
        assert_eq!(debug_config, TimingConfig::debug());

        let demo_config = TimingMode::Demo.to_config();
        assert_eq!(demo_config, TimingConfig::demo());
    }

    #[test]
    fn test_delay_application() {
        let config = TimingConfig::fast();
        
        // These should not panic and should return quickly for fast mode
        let start = std::time::Instant::now();
        config.apply_network_delay();
        config.apply_client_delay();
        let elapsed = start.elapsed();
        
        // Should be very quick for fast mode (delays are 0)
        assert!(elapsed < std::time::Duration::from_millis(10));
    }

    #[test]
    fn test_config_clone_and_equality() {
        let config1 = TimingConfig::debug();
        let config2 = config1.clone();
        
        assert_eq!(config1, config2);
        
        let config3 = TimingConfig::demo();
        assert_ne!(config1, config3);
    }
}
