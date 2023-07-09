use std::time::Instant;

pub struct Measurer(String, Instant);

impl Measurer {
    pub fn new(name: String) -> Self {
        Self(name, Instant::now())
    }
}

impl Drop for Measurer {
    fn drop(&mut self) {
        log::debug!("{} took: {}ms", self.0, self.1.elapsed().as_millis(),);
    }
}

#[macro_export]
macro_rules! measure_this {
    ($($arg:expr),*) => {
        let _measurer = if log::log_enabled!(log::Level::Debug) {
            Some(crate::measure::Measurer::new(format!($($arg),*)))
        } else {
            None
        };
    };
}
