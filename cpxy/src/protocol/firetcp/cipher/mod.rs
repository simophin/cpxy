mod read;
#[cfg(test)]
mod test;
mod write;

pub use read::*;
pub use write::*;

fn cipher_error_to_std(e: cipher::StreamCipherError) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
}
