use std::path::Path;

use async_trait::async_trait;
use tokio::sync::mpsc;

use super::error::PlayerError;
use super::events::PlayerEvent;

#[async_trait]
pub trait PlayerBridge: Send + Sync {
    async fn spawn(&mut self) -> Result<mpsc::Receiver<PlayerEvent>, PlayerError>;
    async fn loadfile(&self, path: &Path) -> Result<(), PlayerError>;
    async fn pause(&self) -> Result<(), PlayerError>;
    async fn play(&self) -> Result<(), PlayerError>;
    async fn seek(&self, seconds: f64) -> Result<(), PlayerError>;
    async fn show_text(&self, message: &str, duration_ms: u32) -> Result<(), PlayerError>;
    async fn get_position(&self) -> Result<Option<f64>, PlayerError>;
    async fn quit(&self) -> Result<(), PlayerError>;
}
