//! Narrow PCM WAV decoding shared by repository-owned presentation sounds.

/// Decodes a mono PCM16 RIFF/WAVE asset to normalized samples.
///
/// These are build-owned files, not an open-ended input format. Refusing any
/// other encoding keeps asset changes visible at build/test time instead of
/// silently growing a second general-purpose audio decoder.
pub fn decode_pcm16_mono(bytes: &[u8], label: &str) -> Result<(Vec<f32>, u32), String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(format!("{label} is not a RIFF/WAVE file"));
    }

    let mut cursor = 12usize;
    let mut format = None;
    let mut data = None;
    while cursor.checked_add(8).is_some_and(|end| end <= bytes.len()) {
        let id = &bytes[cursor..cursor + 4];
        let size = u32::from_le_bytes(
            bytes[cursor + 4..cursor + 8]
                .try_into()
                .expect("four-byte chunk size"),
        ) as usize;
        let start = cursor + 8;
        let end = start
            .checked_add(size)
            .ok_or_else(|| format!("{label} WAV chunk size overflows"))?;
        if end > bytes.len() {
            return Err(format!("{label} WAV chunk is truncated"));
        }
        match id {
            b"fmt " => {
                if size < 16 {
                    return Err(format!("{label} WAV fmt chunk is truncated"));
                }
                let u16_at = |offset: usize| {
                    u16::from_le_bytes(
                        bytes[start + offset..start + offset + 2]
                            .try_into()
                            .expect("two-byte WAV field"),
                    )
                };
                let sample_rate = u32::from_le_bytes(
                    bytes[start + 4..start + 8]
                        .try_into()
                        .expect("four-byte sample rate"),
                );
                format = Some((u16_at(0), u16_at(2), sample_rate, u16_at(14)));
            }
            b"data" => data = Some(&bytes[start..end]),
            _ => {}
        }
        cursor = end + (size & 1);
    }

    let Some((encoding, channels, sample_rate, bits_per_sample)) = format else {
        return Err(format!("{label} WAV has no fmt chunk"));
    };
    if (encoding, channels, bits_per_sample) != (1, 1, 16) || sample_rate == 0 {
        return Err(format!(
            "{label} WAV must be mono PCM16, got encoding {encoding}, channels {channels}, {bits_per_sample} bits"
        ));
    }
    let data = data.ok_or_else(|| format!("{label} WAV has no data chunk"))?;
    if data.len() % 2 != 0 {
        return Err(format!("{label} WAV data has a partial PCM16 sample"));
    }
    let samples = data
        .chunks_exact(2)
        .map(|sample| f32::from(i16::from_le_bytes([sample[0], sample[1]])) / 32_768.0)
        .collect();
    Ok((samples, sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_wav_is_rejected() {
        assert!(decode_pcm16_mono(b"not a wave", "test asset").is_err());
    }
}
