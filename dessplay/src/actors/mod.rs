//! Client actors. Each actor is a tokio task owning its state,
//! communicating via typed message channels (see architecture.md).

pub mod network;
pub mod player;
pub mod sync;
