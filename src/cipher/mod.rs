pub mod stream;

pub trait StreamCipher {
    type Key: AsRef<[u8]>;
    type Iv: AsRef<[u8]>;

    fn new(key: &Self::Key, iv: &Self::Iv) -> Self;

    fn apply_in_place(&mut self, data: &mut [u8]);
}
