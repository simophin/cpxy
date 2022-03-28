use std::{fmt::Debug, sync::Arc};

use derive_more::{AsRef, Deref, DerefMut};
use parking_lot::Mutex;

#[derive(Deref, DerefMut, AsRef)]
pub struct Pooled<T: Default + AsRef<[u8]>> {
    #[deref]
    #[deref_mut]
    #[as_ref(forward)]
    data: T,
    pool: Arc<PoolInner<T>>,
}

impl<T: Default + AsRef<[u8]> + Debug> Debug for Pooled<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&self.data, f)
    }
}

impl<T: Default + AsRef<[u8]>> Drop for Pooled<T> {
    fn drop(&mut self) {
        let mut bufs = self.pool.bufs.lock();
        if bufs.len() < self.pool.max {
            let data = std::mem::take(&mut self.data);
            let insert_point = match bufs
                .binary_search_by(|prob| prob.as_ref().len().cmp(&self.data.as_ref().len()))
            {
                Ok(v) => v,
                Err(v) => v,
            };

            bufs.insert(insert_point, data);
        }
    }
}

struct PoolInner<T> {
    bufs: Mutex<Vec<T>>,
    max: usize,
    factory: fn(usize) -> T,
    pre_recycle: fn(&mut T),
}

impl<T: AsRef<[u8]> + Default> PoolInner<T> {
    pub fn recycle(&self, mut data: T) {
        (self.pre_recycle)(&mut data);
        let mut bufs = self.bufs.lock();
        if bufs.len() < self.max {
            let insert_point =
                match bufs.binary_search_by(|prob| prob.as_ref().len().cmp(&data.as_ref().len())) {
                    Ok(v) => v,
                    Err(v) => v,
                };

            bufs.insert(insert_point, data);
        }
    }
}

#[derive(Clone)]
pub struct Pool<T>(Arc<PoolInner<T>>);

impl<T: AsRef<[u8]> + Default> Pool<T> {
    pub fn new(max: usize, factory: fn(usize) -> T, pre_recycle: fn(&mut T)) -> Self {
        Self(Arc::new(PoolInner {
            bufs: Default::default(),
            max,
            factory,
            pre_recycle,
        }))
    }

    pub fn allocate(&self, len: usize) -> Pooled<T> {
        let mut bufs = self.0.bufs.lock();
        let data = match bufs.binary_search_by(|probe| probe.as_ref().len().cmp(&len)) {
            Ok(index) => bufs.remove(index),
            Err(index) if index < bufs.len() - 1 => bufs.remove(index + 1),
            Err(_) => (self.0.factory)(len),
        };
        Pooled {
            data,
            pool: self.0.clone(),
        }
    }

    pub fn recycle(&self, data: T) {
        self.0.recycle(data)
    }
}

impl Pool<Vec<u8>> {
    fn clear_vec(b: &mut Vec<u8>) {
        unsafe { b.set_len(b.capacity()) }
    }

    fn new_vec(cap: usize) -> Vec<u8> {
        vec![0u8; cap]
    }

    pub fn new_with_vec(max: usize) -> Self {
        Self::new(max, Self::new_vec, Self::clear_vec)
    }
}
