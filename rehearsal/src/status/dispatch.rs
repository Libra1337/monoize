use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchPath {
    HttpInitial,
    HttpRetry,
    ProviderFailForward,
    WebSocket,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchError {
    InvalidPermitCount,
    Closed,
}

#[derive(Debug)]
pub struct DispatchGate {
    semaphore: Arc<Semaphore>,
    current: Arc<AtomicUsize>,
    maximum: Arc<AtomicUsize>,
}

#[derive(Debug)]
pub struct DispatchGuard {
    _permit: OwnedSemaphorePermit,
    current: Arc<AtomicUsize>,
    pub path: DispatchPath,
}

impl DispatchGate {
    pub fn new(permits: usize) -> Result<Self, DispatchError> {
        if permits == 0 {
            return Err(DispatchError::InvalidPermitCount);
        }
        Ok(Self {
            semaphore: Arc::new(Semaphore::new(permits)),
            current: Arc::new(AtomicUsize::new(0)),
            maximum: Arc::new(AtomicUsize::new(0)),
        })
    }

    pub async fn acquire(&self, path: DispatchPath) -> Result<DispatchGuard, DispatchError> {
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| DispatchError::Closed)?;
        let current = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(current, Ordering::SeqCst);
        Ok(DispatchGuard {
            _permit: permit,
            current: self.current.clone(),
            path,
        })
    }

    pub async fn dispatch<F, Fut>(
        &self,
        path: DispatchPath,
        operation: F,
    ) -> Result<(), DispatchError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ()>,
    {
        let _guard = self.acquire(path).await?;
        operation().await;
        Ok(())
    }

    pub fn current(&self) -> usize {
        self.current.load(Ordering::SeqCst)
    }

    pub fn maximum_observed(&self) -> usize {
        self.maximum.load(Ordering::SeqCst)
    }
}

impl Drop for DispatchGuard {
    fn drop(&mut self) {
        self.current.fetch_sub(1, Ordering::SeqCst);
    }
}
