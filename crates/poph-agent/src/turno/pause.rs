use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

pub type FuturePause = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

pub trait Pause: Send + Sync + 'static {
    fn wait(&self, duration: Duration) -> FuturePause;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PauseReal;

impl Pause for PauseReal {
    fn wait(&self, duration: Duration) -> FuturePause {
        if duration.is_zero() {
            return Box::pin(async {});
        }
        Box::pin(tokio::time::sleep(duration))
    }
}
