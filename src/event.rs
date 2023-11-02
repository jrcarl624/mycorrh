use std::{
    result::Result,
    sync::Arc,
};

use tokio::{
    sync::RwLock,
    task::JoinHandle,
};

pub trait ScEventEmitter: EventLoop {
    type Event: Send + Sync + std::fmt::Debug;

    fn emit(&self, event: Self::Event) -> Result<(), Self::Err>;
}

pub trait McEventEmitter<T>: EventLoop {
    fn emit(&self, data: T) -> Result<(), Self::Err>;

    fn on(&self, callback: dyn FnOnce(T) -> Result<(), Self::Err>) -> Result<(), Self::Err>;
}

#[async_trait::async_trait]
pub trait EventLoop {
    type Close: Send + Sync + std::fmt::Debug + std::fmt::Display;
    type Err: Send + Sync + std::error::Error;

    fn event_loop_join_handle(&self) -> &Arc<RwLock<JoinHandle<Result<Self::Close, Self::Err>>>>;

    fn is_closed(&self) -> bool;

    /// If Err is returned, the event loop is already closed.
    async fn close(&self, reason: Self::Close) -> Result<(), Self::Err>;
}
