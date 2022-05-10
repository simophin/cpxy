use bytes::Bytes;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BytesRef<'a> {
    Ref(&'a [u8]),
    Bytes(Bytes),
}

impl<'a> From<&'a [u8]> for BytesRef<'a> {
    fn from(v: &'a [u8]) -> Self {
        BytesRef::Ref(v)
    }
}

impl From<Bytes> for BytesRef<'static> {
    fn from(v: Bytes) -> Self {
        BytesRef::Bytes(v)
    }
}

impl<'a> AsRef<[u8]> for BytesRef<'a> {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Ref(v) => *v,
            Self::Bytes(b) => b.as_ref(),
        }
    }
}

impl<'a> Into<Bytes> for BytesRef<'a> {
    fn into(self) -> Bytes {
        match self {
            Self::Ref(v) => Bytes::copy_from_slice(v),
            Self::Bytes(b) => b,
        }
    }
}
