mod read;
#[cfg(test)]
mod test;
mod write;

use cipher::StreamCipherError;
pub use read::*;
pub use write::*;

fn cipher_error_to_std(e: StreamCipherError) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
}
