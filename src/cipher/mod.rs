pub mod chacha20;
pub mod stream;

pub trait StreamCipher {
    type Key: AsRef<[u8]>;
    type Iv: AsRef<[u8]>;

    fn new(key: &Self::Key, iv: &Self::Iv) -> Self;

    fn apply_in_place(&mut self, data: &mut [u8]);

    fn rand_key_iv() -> (Self::Key, Self::Iv)
    where
        Self::Key: Default + AsMut<[u8]>,
        Self::Iv: Default + AsMut<[u8]>,
    {
        let mut key = Self::Key::default();
        let mut iv = Self::Iv::default();
        orion::util::secure_rand_bytes(key.as_mut()).unwrap();
        orion::util::secure_rand_bytes(iv.as_mut()).unwrap();
        (key, iv)
    }
}
