//! Message bus implementation for event-driven communication
//! 
//! This module provides a synchronous message bus that enables loose coupling
//! between components through event publishing and subscription.

use crate::{Result, Error};
use super::{Event, EventHandler};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;

/// Message bus for event-driven communication
pub struct MessageBus {
    /// Event handlers registered for different event types
    handlers: Arc<Mutex<Vec<Box<dyn EventHandler>>>>,
    /// Event queue for processing
    event_queue: Arc<Mutex<Vec<Event>>>,
}

impl MessageBus {
    /// Create a new message bus
    pub fn new() -> Self {
        Self {
            handlers: Arc::new(Mutex::new(Vec::new())),
            event_queue: Arc::new(Mutex::new(Vec::new())),
        }
    }
    
    /// Subscribe an event handler to the bus
    pub fn subscribe(&self, handler: Box<dyn EventHandler>) {
        self.handlers.lock().unwrap().push(handler);
    }
    
    /// Publish an event to all subscribers
    pub fn publish(&self, event: Event) -> Result<()> {
        log::debug!("Publishing event to {} handlers", self.handler_count());
        log::trace!("Event details: {:?}", event);
        
        let mut handlers = self.handlers.lock().unwrap();
        for (i, handler) in handlers.iter_mut().enumerate() {
            if let Err(e) = handler.handle_event(event.clone()) {
                log::error!("Handler {} failed to process event: {}", i, e);
                return Err(e);
            }
        }
        
        log::trace!("Event successfully processed by all handlers");
        Ok(())
    }
    
    /// Queue an event for later processing
    pub fn queue_event(&self, event: Event) {
        self.event_queue.lock().unwrap().push(event);
    }
    
    /// Process all queued events
    pub fn process_queued_events(&self) -> Result<()> {
        let events = {
            let mut queue = self.event_queue.lock().unwrap();
            let events = queue.clone();
            queue.clear();
            events
        };
        
        for event in events {
            self.publish(event)?;
        }
        
        Ok(())
    }
    
    /// Get the number of registered handlers
    pub fn handler_count(&self) -> usize {
        self.handlers.lock().unwrap().len()
    }
    
    /// Get the number of queued events
    pub fn queued_event_count(&self) -> usize {
        self.event_queue.lock().unwrap().len()
    }
}

impl Default for MessageBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Mock event handler for testing
#[derive(Debug)]
pub struct MockEventHandler {
    /// Events that have been handled
    handled_events: Arc<Mutex<Vec<Event>>>,
    /// Whether to return an error on handle_event
    should_error: bool,
}

impl MockEventHandler {
    /// Create a new mock event handler
    pub fn new() -> Self {
        Self {
            handled_events: Arc::new(Mutex::new(Vec::new())),
            should_error: false,
        }
    }
    
    /// Create a mock handler that returns errors
    pub fn with_error() -> Self {
        Self {
            handled_events: Arc::new(Mutex::new(Vec::new())),
            should_error: true,
        }
    }
    
    /// Get all handled events
    pub fn get_handled_events(&self) -> Vec<Event> {
        self.handled_events.lock().unwrap().clone()
    }
    
    /// Clear handled events
    pub fn clear_handled_events(&self) {
        self.handled_events.lock().unwrap().clear();
    }
    
    /// Get the number of handled events
    pub fn handled_event_count(&self) -> usize {
        self.handled_events.lock().unwrap().len()
    }
}

impl EventHandler for MockEventHandler {
    fn handle_event(&mut self, event: Event) -> Result<()> {
        if self.should_error {
            return Err(Error::Network("Mock handler error".to_string()));
        }
        
        self.handled_events.lock().unwrap().push(event);
        Ok(())
    }
}

/// Event counter handler for testing
#[derive(Debug)]
pub struct EventCounterHandler {
    /// Count of events by type
    counts: Arc<Mutex<HashMap<String, usize>>>,
}

impl EventCounterHandler {
    /// Create a new event counter handler
    pub fn new() -> Self {
        Self {
            counts: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    
    /// Get count for a specific event type
    pub fn get_count(&self, event_type: &str) -> usize {
        self.counts.lock().unwrap().get(event_type).copied().unwrap_or(0)
    }
    
    /// Get all counts
    pub fn get_all_counts(&self) -> HashMap<String, usize> {
        self.counts.lock().unwrap().clone()
    }
    
    /// Clear all counts
    pub fn clear_counts(&self) {
        self.counts.lock().unwrap().clear();
    }
    
    /// Get event type name
    fn event_type_name(&self, event: &Event) -> String {
        match event {
            Event::Raft(_) => "Raft".to_string(),
            Event::Client(_) => "Client".to_string(),
            Event::Network(_) => "Network".to_string(),
            Event::Timer(_) => "Timer".to_string(),
        }
    }
}

impl EventHandler for EventCounterHandler {
    fn handle_event(&mut self, event: Event) -> Result<()> {
        let event_type = self.event_type_name(&event);
        let mut counts = self.counts.lock().unwrap();
        *counts.entry(event_type).or_insert(0) += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{RaftEvent, ClientEvent, NetworkEvent, TimerEvent};

    #[test]
    fn test_message_bus_creation() {
        let bus = MessageBus::new();
        assert_eq!(bus.handler_count(), 0);
        assert_eq!(bus.queued_event_count(), 0);
    }

    #[test]
    fn test_event_subscription_and_publishing() {
        let bus = MessageBus::new();
        let handler = MockEventHandler::new();
        
        bus.subscribe(Box::new(handler));
        assert_eq!(bus.handler_count(), 1);
        
        let event = Event::Raft(RaftEvent::ElectionTimeout);
        bus.publish(event.clone()).unwrap();
        
        // Note: In this simple implementation, we can't easily verify
        // that the handler received the event because it's moved into the bus
    }

    #[test]
    fn test_event_queuing() {
        let bus = MessageBus::new();
        
        let event1 = Event::Client(ClientEvent::Get { key: "test".to_string() });
        let event2 = Event::Timer(TimerEvent::ElectionTimer);
        
        bus.queue_event(event1);
        bus.queue_event(event2);
        
        assert_eq!(bus.queued_event_count(), 2);
        
        // Add a handler to process events
        let handler = MockEventHandler::new();
        bus.subscribe(Box::new(handler));
        
        bus.process_queued_events().unwrap();
        assert_eq!(bus.queued_event_count(), 0);
    }

    #[test]
    fn test_mock_event_handler() {
        let mut handler = MockEventHandler::new();
        
        let event1 = Event::Network(NetworkEvent::ConnectionEstablished { node_id: 1 });
        let event2 = Event::Raft(RaftEvent::HeartbeatTimeout);
        
        handler.handle_event(event1.clone()).unwrap();
        handler.handle_event(event2.clone()).unwrap();
        
        assert_eq!(handler.handled_event_count(), 2);
        
        let handled = handler.get_handled_events();
        assert_eq!(handled.len(), 2);
        
        handler.clear_handled_events();
        assert_eq!(handler.handled_event_count(), 0);
    }

    #[test]
    fn test_mock_handler_with_error() {
        let mut handler = MockEventHandler::with_error();
        let event = Event::Timer(TimerEvent::HeartbeatTimer);
        
        let result = handler.handle_event(event);
        assert!(result.is_err());
    }

    #[test]
    fn test_event_counter_handler() {
        let mut handler = EventCounterHandler::new();
        
        let raft_event = Event::Raft(RaftEvent::ElectionTimeout);
        let client_event = Event::Client(ClientEvent::Put { 
            key: "key".to_string(), 
            value: b"value".to_vec() 
        });
        let network_event = Event::Network(NetworkEvent::ConnectionLost { node_id: 2 });
        
        handler.handle_event(raft_event).unwrap();
        handler.handle_event(client_event).unwrap();
        handler.handle_event(network_event).unwrap();
        
        assert_eq!(handler.get_count("Raft"), 1);
        assert_eq!(handler.get_count("Client"), 1);
        assert_eq!(handler.get_count("Network"), 1);
        assert_eq!(handler.get_count("Timer"), 0);
        
        let all_counts = handler.get_all_counts();
        assert_eq!(all_counts.len(), 3);
        
        handler.clear_counts();
        assert_eq!(handler.get_count("Raft"), 0);
    }

    #[test]
    fn test_message_bus_error_handling() {
        let bus = MessageBus::new();
        let error_handler = MockEventHandler::with_error();
        
        bus.subscribe(Box::new(error_handler));
        
        let event = Event::Raft(RaftEvent::ElectionTimeout);
        let result = bus.publish(event);
        
        assert!(result.is_err());
    }
}
