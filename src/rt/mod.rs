#[cfg(feature = "tokio")]
mod tokio;

#[cfg(feature = "smol")]
mod smol;

#[cfg(feature = "smol")]
pub use self::smol::*;

#[cfg(all(feature = "tokio", not(feature = "smol")))]
pub use self::tokio::*;
