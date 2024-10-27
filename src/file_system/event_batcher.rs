use std::time::Duration;

use tokio::{sync::mpsc, time::Instant};
use tokio::time::{interval_at, MissedTickBehavior};
use tokio::sync::RwLock;
use std::sync::Arc;

use crate::file_system::FileEvent;

// New struct to handle event batching
#[derive(Debug)]
pub struct EventBatcher {
    batch_size: usize,
    batch_timeout: Duration,
    events: Vec<FileEvent>,
    last_emit: Instant,
    event_sender: mpsc::Sender<Vec<FileEvent>>,
}

impl EventBatcher {
    pub fn new(
        batch_size: usize, 
        batch_timeout: Duration, 
        event_sender: mpsc::Sender<Vec<FileEvent>>
    ) -> Self {
        Self {
            batch_size,
            batch_timeout,
            events: Vec::with_capacity(batch_size),
            last_emit: Instant::now(),
            event_sender,
        }
    }

    pub async fn add_event(&mut self, event: FileEvent) {
        self.events.push(event);
        
        if self.should_emit() {
            self.emit_batch().await;
        }
    }

    fn should_emit(&self) -> bool {
        self.events.len() >= self.batch_size || 
        self.last_emit.elapsed() >= self.batch_timeout
    }

    async fn emit_batch(&mut self) {
        if self.events.is_empty() {
            return;
        }

        let batch = std::mem::replace(&mut self.events, Vec::with_capacity(self.batch_size));
        println!("Emitting batch of {} events", batch.len());
        
        if let Err(e) = self.event_sender.send(batch).await {
            eprintln!("Failed to send event batch: {}", e);
        }
        self.last_emit = Instant::now();
    }
}
// Separate function for spawning the timeout checker

pub fn spawn_timeout_checker(batcher: Arc<RwLock<EventBatcher>>) {
    tokio::spawn(async move {
        let mut interval = interval_at(
            Instant::now() + batcher.read().await.batch_timeout,
            batcher.read().await.batch_timeout
        );
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            interval.tick().await;
            let mut locked_batcher = batcher.write().await;
            if !locked_batcher.events.is_empty() && 
               locked_batcher.last_emit.elapsed() >= locked_batcher.batch_timeout {
                locked_batcher.emit_batch().await;
            }
        }
    });
}