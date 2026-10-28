use crate::Error;
use core::time::Duration;

#[allow(async_fn_in_trait)]
pub trait Transport {
    async fn flush(&mut self, timeout: Duration) -> Result<usize, Error>;
    async fn receive_frame(&mut self, timeout: Duration) -> Result<&mut [u8], Error>;
    async fn transmit_frame(&mut self, frame: &[u8]) -> Result<(), Error>;
}

#[cfg(feature = "no-std")]
mod embedded;
#[cfg(feature = "std")]
mod serialport;
#[cfg(feature = "std")]
mod tokio_channels;

#[cfg(feature = "no-std")]
pub use embedded::EioTransport;
#[cfg(feature = "std")]
pub use serialport::SerialTransport;
#[cfg(feature = "std")]
pub use tokio_channels::TokioChannelTransport;
