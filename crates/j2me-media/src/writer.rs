//! Minimal big-endian writer used by the lossless SMAF encoder.

#[derive(Debug, Default, Clone)]
pub(crate) struct Writer {
    data: Vec<u8>,
}

impl Writer {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    pub(crate) fn u8(&mut self, value: u8) -> &mut Self {
        self.data.push(value);
        self
    }

    pub(crate) fn u32(&mut self, value: u32) -> &mut Self {
        self.data.extend_from_slice(&value.to_be_bytes());
        self
    }

    pub(crate) fn bytes(&mut self, value: &[u8]) -> &mut Self {
        self.data.extend_from_slice(value);
        self
    }
}
