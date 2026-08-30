use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    UnexpectedEof {
        offset: usize,
        needed: usize,
        remaining: usize,
    },
    NegativeCount {
        offset: usize,
        field: &'static str,
        value: i64,
    },
    InvalidUtf8 {
        offset: usize,
        field: &'static str,
    },
    InvalidMagic {
        format: &'static str,
    },
    InvalidValue {
        offset: usize,
        field: &'static str,
        value: u64,
    },
    MissingTerminator {
        offset: usize,
        field: &'static str,
    },
    ChecksumMismatch {
        offset: usize,
        stored: u32,
        computed: u32,
    },
    Decompression {
        offset: usize,
    },
    LengthMismatch {
        declared: usize,
        actual: usize,
    },
    TrailingBytes {
        offset: usize,
        remaining: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof {
                offset,
                needed,
                remaining,
            } => write!(
                formatter,
                "unexpected end at 0x{offset:x}: needed {needed} bytes, found {remaining}"
            ),
            Self::NegativeCount {
                offset,
                field,
                value,
            } => write!(formatter, "negative {field} count {value} at 0x{offset:x}"),
            Self::InvalidUtf8 { offset, field } => {
                write!(formatter, "invalid UTF-8 in {field} at 0x{offset:x}")
            }
            Self::InvalidMagic { format } => write!(formatter, "invalid {format} signature"),
            Self::InvalidValue {
                offset,
                field,
                value,
            } => write!(formatter, "invalid {field} value {value} at 0x{offset:x}"),
            Self::MissingTerminator { offset, field } => {
                write!(
                    formatter,
                    "missing NUL terminator for {field} at 0x{offset:x}"
                )
            }
            Self::ChecksumMismatch {
                offset,
                stored,
                computed,
            } => write!(
                formatter,
                "checksum mismatch at 0x{offset:x}: stored {stored:08x}, computed {computed:08x}"
            ),
            Self::Decompression { offset } => {
                write!(formatter, "invalid zlib stream at 0x{offset:x}")
            }
            Self::LengthMismatch { declared, actual } => write!(
                formatter,
                "length prefix declares {declared} payload bytes, file contains {actual}"
            ),
            Self::TrailingBytes { offset, remaining } => {
                write!(formatter, "{remaining} trailing bytes at 0x{offset:x}")
            }
        }
    }
}

impl std::error::Error for Error {}
