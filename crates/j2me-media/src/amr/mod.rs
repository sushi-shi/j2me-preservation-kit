//! RFC 4867 AMR-NB storage parsing and 3GPP TS 26.073 speech decoding.
//!
//! Java ME games commonly carry the single-channel storage format: the magic
//! `#!AMR\n` followed by
//! frames of `[header octet][mode-sized payload]`, 20 ms of speech each.
//! The header octet is `P FT FT FT FT Q P P`; the three padding bits must
//! be zero (`RFC 4867 §5.1`).
//!
//! [`parse`] validates the container and recovers frame timing;
//! [`AmrTrack::decode`] turns it into 8 kHz signed 16-bit PCM. The decoder
//! lives in [`mr122`] and is integer-only, so native, WASM, and the site
//! exporter all produce the same samples.
//!
//! # Scope, and what "unsupported" means here
//!
//! Only MR122, the 12.2 kbit/s mode, is decoded. The implementation was
//! validated against the fixed-point reference on the recovery corpus; the
//! seven other rates have no retained evidence here. [`AmrTrack::decode`] therefore refuses
//! any other frame type by name rather than substituting silence or
//! guessing, so a future build that ships a different rate fails loudly
//! instead of playing something wrong.

pub mod fixed;
pub mod mr122;
pub mod tables;

/// The storage-format magic for single-channel AMR-NB.
pub const MAGIC: &[u8] = b"#!AMR\n";

/// Milliseconds of speech per storage frame.
pub const FRAME_MILLIS: u32 = 20;

/// AMR-NB is narrowband: 8 kHz, one channel.
pub const SAMPLE_RATE: u32 = 8000;

/// The frame type of the only mode this decoder accepts.
pub const MR122: u8 = 7;

/// One storage frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmrFrame {
    /// Frame type 0..=8 or 15 (`NO_DATA`); 0..=7 are the speech modes
    /// MR475..MR122, 8 is SID comfort noise.
    pub frame_type: u8,
    /// The frame-quality bit (1 = good).
    pub quality_ok: bool,
    /// The packed class-A/B/C bits, without the header octet.
    pub payload: Vec<u8>,
}

/// A parsed AMR-NB storage container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmrTrack {
    pub frames: Vec<AmrFrame>,
}

impl AmrTrack {
    /// Total speech duration: every frame, including SID and `NO_DATA`,
    /// covers 20 ms.
    pub fn duration_millis(&self) -> u32 {
        self.frames.len() as u32 * FRAME_MILLIS
    }

    /// Whether every frame uses one speech mode.
    pub fn uniform_mode(&self) -> Option<u8> {
        let first = self.frames.first()?.frame_type;
        self.frames
            .iter()
            .all(|frame| frame.frame_type == first)
            .then_some(first)
    }

    /// Decodes the whole track to 8 kHz signed 16-bit PCM.
    ///
    /// Returns exactly 160 samples per frame. Fails with
    /// [`AmrError::UnsupportedMode`] on the first frame that is not MR122,
    /// naming the frame type, rather than filling the gap with silence.
    pub fn decode(&self) -> Result<Vec<i16>, AmrError> {
        let mut decoder = mr122::Mr122Decoder::new();
        let mut samples = Vec::with_capacity(self.frames.len() * mr122::FRAME_SAMPLES);
        for (index, frame) in self.frames.iter().enumerate() {
            if frame.frame_type != MR122 {
                return Err(AmrError::UnsupportedMode {
                    frame: index,
                    frame_type: frame.frame_type,
                });
            }
            // `parse` has already pinned the payload length per frame type,
            // so this cannot fail for a parsed track.
            let payload: &[u8; 31] =
                frame
                    .payload
                    .as_slice()
                    .try_into()
                    .map_err(|_| AmrError::UnsupportedMode {
                        frame: index,
                        frame_type: frame.frame_type,
                    })?;
            samples.extend_from_slice(&decoder.decode_frame(payload));
        }
        Ok(samples)
    }

    /// Decodes to normalised floating-point samples for host mixing.
    ///
    /// The scale is the one every other sink in this crate speaks: full
    /// scale is 1.0. `i16::MIN` maps just past -1.0, exactly as dividing by
    /// 32768 implies; nothing clips because the host gain only ever scales
    /// down.
    pub fn decode_f32(&self) -> Result<Vec<f32>, AmrError> {
        Ok(self
            .decode()?
            .into_iter()
            .map(|sample| f32::from(sample) / 32768.0)
            .collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AmrError {
    /// The blob does not begin with `#!AMR\n`.
    BadMagic,
    /// The magic is present but no frames follow.
    Empty,
    /// A frame header octet has nonzero padding bits or a reserved frame
    /// type (9..=14).
    BadFrameHeader { offset: usize, octet: u8 },
    /// A frame's payload runs past the end of the blob.
    Truncated { offset: usize, frame_type: u8 },
    /// The container is well formed but carries a mode this decoder does not
    /// implement. Parsing still succeeds; only [`AmrTrack::decode`] refuses.
    UnsupportedMode { frame: usize, frame_type: u8 },
}

impl std::fmt::Display for AmrError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AmrError::BadMagic => write!(formatter, "not an AMR-NB storage blob (bad magic)"),
            AmrError::Empty => write!(formatter, "AMR-NB blob has no frames"),
            AmrError::BadFrameHeader { offset, octet } => write!(
                formatter,
                "invalid AMR frame header {octet:#04x} at offset {offset}"
            ),
            AmrError::Truncated { offset, frame_type } => write!(
                formatter,
                "AMR frame (type {frame_type}) at offset {offset} is truncated"
            ),
            AmrError::UnsupportedMode { frame, frame_type } => write!(
                formatter,
                "AMR frame {frame} uses frame type {frame_type}; only MR122 \
                 (type {MR122}, 12.2 kbit/s) is decoded"
            ),
        }
    }
}

impl std::error::Error for AmrError {}

/// Payload octets per frame type (without the header octet), from the
/// RFC 4867 storage table: MR475..MR122 speech, SID, and `NO_DATA`.
fn payload_len(frame_type: u8) -> Option<usize> {
    match frame_type {
        0 => Some(12), // MR475
        1 => Some(13), // MR515
        2 => Some(15), // MR59
        3 => Some(17), // MR67
        4 => Some(19), // MR74
        5 => Some(20), // MR795
        6 => Some(26), // MR102
        7 => Some(31), // MR122
        8 => Some(5),  // SID
        15 => Some(0), // NO_DATA
        _ => None,     // 9..=14 reserved
    }
}

/// Parses one single-channel storage blob, rejecting malformed or truncated
/// input without panicking.
pub fn parse(bytes: &[u8]) -> Result<AmrTrack, AmrError> {
    let body = bytes.strip_prefix(MAGIC).ok_or(AmrError::BadMagic)?;
    if body.is_empty() {
        return Err(AmrError::Empty);
    }
    let mut frames = Vec::new();
    let mut offset = 0usize;
    while offset < body.len() {
        let octet = body[offset];
        // P(1) FT(4) Q(1) P(2): the three padding bits must be zero.
        if octet & 0x83 != 0 {
            return Err(AmrError::BadFrameHeader {
                offset: MAGIC.len() + offset,
                octet,
            });
        }
        let frame_type = (octet >> 3) & 0x0f;
        let Some(length) = payload_len(frame_type) else {
            return Err(AmrError::BadFrameHeader {
                offset: MAGIC.len() + offset,
                octet,
            });
        };
        let start = offset + 1;
        let end = start + length;
        if end > body.len() {
            return Err(AmrError::Truncated {
                offset: MAGIC.len() + offset,
                frame_type,
            });
        }
        frames.push(AmrFrame {
            frame_type,
            quality_ok: octet & 0x04 != 0,
            payload: body[start..end].to_vec(),
        });
        offset = end;
    }
    Ok(AmrTrack { frames })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(frame_type: u8, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![(frame_type << 3) | 0x04];
        bytes.extend_from_slice(payload);
        bytes
    }

    fn blob(frames: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        for frame in frames {
            bytes.extend_from_slice(frame);
        }
        bytes
    }

    #[test]
    fn a_synthetic_mr122_stream_parses_with_exact_timing() {
        let bytes = blob(&[frame(7, &[0u8; 31]), frame(7, &[1u8; 31])]);
        let track = parse(&bytes).unwrap();
        assert_eq!(track.frames.len(), 2);
        assert_eq!(track.duration_millis(), 40);
        assert_eq!(track.uniform_mode(), Some(7));
        assert!(track.frames.iter().all(|frame| frame.quality_ok));
    }

    #[test]
    fn every_speech_mode_and_no_data_parse() {
        let sizes = [12, 13, 15, 17, 19, 20, 26, 31, 5, 0];
        let types = [0u8, 1, 2, 3, 4, 5, 6, 7, 8, 15];
        let frames: Vec<Vec<u8>> = types
            .iter()
            .zip(sizes)
            .map(|(&frame_type, size)| frame(frame_type, &vec![0u8; size]))
            .collect();
        let track = parse(&blob(&frames)).unwrap();
        assert_eq!(track.frames.len(), 10);
        assert_eq!(track.uniform_mode(), None);
    }

    #[test]
    fn malformed_input_is_rejected_without_panicking() {
        assert_eq!(parse(b""), Err(AmrError::BadMagic));
        assert_eq!(parse(b"#!AMR"), Err(AmrError::BadMagic));
        assert_eq!(parse(b"RIFF...."), Err(AmrError::BadMagic));
        assert_eq!(parse(MAGIC), Err(AmrError::Empty));
        // Reserved frame type 9.
        assert_eq!(
            parse(&blob(&[frame(9, &[0u8; 4])])),
            Err(AmrError::BadFrameHeader {
                offset: 6,
                octet: (9 << 3) | 0x04
            })
        );
        // Nonzero padding bit.
        let mut padded = MAGIC.to_vec();
        padded.push(0x81);
        assert_eq!(
            parse(&padded),
            Err(AmrError::BadFrameHeader {
                offset: 6,
                octet: 0x81
            })
        );
        // Truncated MR122 payload.
        assert_eq!(
            parse(&blob(&[frame(7, &[0u8; 30])])),
            Err(AmrError::Truncated {
                offset: 6,
                frame_type: 7
            })
        );
    }
}
