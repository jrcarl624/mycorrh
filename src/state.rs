/// Used for shared state between handles and event loops.
/// Storage for Atomics and Locks (as needed).
/// So when this is cloned only the 16 bytes of the Arc are copied.
pub trait State {
    type State: Send + Sync + std::fmt::Debug;
    fn state(&self) -> &std::sync::Arc<Self::State>;
}
