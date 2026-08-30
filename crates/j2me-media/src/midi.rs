//! Bounded Standard MIDI File parsing.
//!
//! This parser recovers what Java ME playback adapters need — an
//! absolute-microsecond note list with per-note program numbers and the
//! tempo map applied — and rejects malformed or truncated input without
//! panicking. SMPTE divisions and format 2 are refused rather than guessed at.

/// One sounding note, in absolute microseconds from the song start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiNote {
    pub start_us: u64,
    pub duration_us: u64,
    /// MIDI channel 0..=15; channel 9 is percussion.
    pub channel: u8,
    /// Key number 0..=127.
    pub key: u8,
    /// Note-on velocity 1..=127.
    pub velocity: u8,
    /// The General MIDI program active on the channel at note-on (0 when
    /// none was set).
    pub program: u8,
}

/// A parsed song: the note list plus its structural timing facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiSong {
    pub format: u16,
    pub track_count: u16,
    pub ticks_per_quarter: u16,
    /// Notes ordered by start time.
    pub notes: Vec<MidiNote>,
    /// The end of the last note.
    pub duration_us: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MidiError {
    /// Missing or malformed `MThd` header (including SMPTE-free zero
    /// divisions).
    BadHeader,
    /// Format 2 (independent patterns) is refused.
    UnsupportedFormat(u16),
    /// SMPTE divisions are refused.
    SmpteDivision,
    /// A track chunk is missing its `MTrk` magic or its length overruns.
    BadTrackChunk { track: u16 },
    /// An event ran past the end of its track chunk.
    Truncated { track: u16 },
    /// A data byte carried the status bit, or an event had no running
    /// status to inherit.
    BadEvent { track: u16, offset: usize },
}

impl std::fmt::Display for MidiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MidiError::BadHeader => write!(formatter, "not a standard MIDI file (bad MThd)"),
            MidiError::UnsupportedFormat(format) => {
                write!(formatter, "SMF format {format} is not supported")
            }
            MidiError::SmpteDivision => write!(formatter, "SMPTE division is not supported"),
            MidiError::BadTrackChunk { track } => {
                write!(formatter, "track {track} has a malformed chunk")
            }
            MidiError::Truncated { track } => write!(formatter, "track {track} is truncated"),
            MidiError::BadEvent { track, offset } => {
                write!(formatter, "track {track} has a malformed event at {offset}")
            }
        }
    }
}

impl std::error::Error for MidiError {}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
    track: u16,
}

impl<'a> Cursor<'a> {
    fn u8(&mut self) -> Result<u8, MidiError> {
        let byte = *self
            .bytes
            .get(self.position)
            .ok_or(MidiError::Truncated { track: self.track })?;
        self.position += 1;
        Ok(byte)
    }

    fn data_byte(&mut self) -> Result<u8, MidiError> {
        let offset = self.position;
        let byte = self.u8()?;
        if byte & 0x80 != 0 {
            return Err(MidiError::BadEvent {
                track: self.track,
                offset,
            });
        }
        Ok(byte)
    }

    fn variable_length(&mut self) -> Result<u32, MidiError> {
        let mut value: u32 = 0;
        for _ in 0..4 {
            let byte = self.u8()?;
            value = (value << 7) | u32::from(byte & 0x7f);
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(MidiError::BadEvent {
            track: self.track,
            offset: self.position,
        })
    }

    fn skip(&mut self, count: usize) -> Result<(), MidiError> {
        if self.position + count > self.bytes.len() {
            return Err(MidiError::Truncated { track: self.track });
        }
        self.position += count;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
enum TrackEvent {
    NoteOn { channel: u8, key: u8, velocity: u8 },
    NoteOff { channel: u8, key: u8 },
    Program { channel: u8, program: u8 },
    Tempo { us_per_quarter: u32 },
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

/// Parses one SMF blob into an absolute-time note list.
pub fn parse(bytes: &[u8]) -> Result<MidiSong, MidiError> {
    if bytes.get(0..4) != Some(b"MThd") {
        return Err(MidiError::BadHeader);
    }
    let header_len = read_u32(bytes, 4).ok_or(MidiError::BadHeader)? as usize;
    if header_len < 6 {
        return Err(MidiError::BadHeader);
    }
    let format = read_u16(bytes, 8).ok_or(MidiError::BadHeader)?;
    let track_count = read_u16(bytes, 10).ok_or(MidiError::BadHeader)?;
    let division = read_u16(bytes, 12).ok_or(MidiError::BadHeader)?;
    if format > 1 {
        return Err(MidiError::UnsupportedFormat(format));
    }
    if division & 0x8000 != 0 {
        return Err(MidiError::SmpteDivision);
    }
    if division == 0 {
        return Err(MidiError::BadHeader);
    }

    let mut events: Vec<(u64, usize, TrackEvent)> = Vec::new();
    let mut sequence = 0usize;
    let mut position = 8 + header_len;
    for track in 0..track_count {
        if bytes.get(position..position + 4) != Some(b"MTrk") {
            return Err(MidiError::BadTrackChunk { track });
        }
        let length =
            read_u32(bytes, position + 4).ok_or(MidiError::BadTrackChunk { track })? as usize;
        let body_start = position + 8;
        let body_end = body_start + length;
        if body_end > bytes.len() {
            return Err(MidiError::BadTrackChunk { track });
        }
        let mut cursor = Cursor {
            bytes: &bytes[..body_end],
            position: body_start,
            track,
        };
        let mut tick: u64 = 0;
        let mut running_status: Option<u8> = None;
        while cursor.position < body_end {
            tick += u64::from(cursor.variable_length()?);
            let lead_offset = cursor.position;
            let lead = cursor.u8()?;
            let status = if lead & 0x80 != 0 {
                lead
            } else {
                // Running status: rewind the data byte.
                cursor.position = lead_offset;
                running_status.ok_or(MidiError::BadEvent {
                    track,
                    offset: lead_offset,
                })?
            };
            match status {
                0xff => {
                    running_status = None;
                    let meta = cursor.u8()?;
                    let length = cursor.variable_length()? as usize;
                    let data_start = cursor.position;
                    cursor.skip(length)?;
                    if meta == 0x51 && length == 3 {
                        let data = &cursor.bytes[data_start..data_start + 3];
                        let us_per_quarter = (u32::from(data[0]) << 16)
                            | (u32::from(data[1]) << 8)
                            | u32::from(data[2]);
                        events.push((tick, sequence, TrackEvent::Tempo { us_per_quarter }));
                        sequence += 1;
                    }
                }
                0xf0 | 0xf7 => {
                    running_status = None;
                    let length = cursor.variable_length()? as usize;
                    cursor.skip(length)?;
                }
                0xf1..=0xf6 | 0xf8..=0xfe => {
                    return Err(MidiError::BadEvent {
                        track,
                        offset: lead_offset,
                    });
                }
                _ => {
                    running_status = Some(status);
                    let channel = status & 0x0f;
                    match status & 0xf0 {
                        0x80 => {
                            let key = cursor.data_byte()?;
                            let _velocity = cursor.data_byte()?;
                            events.push((tick, sequence, TrackEvent::NoteOff { channel, key }));
                            sequence += 1;
                        }
                        0x90 => {
                            let key = cursor.data_byte()?;
                            let velocity = cursor.data_byte()?;
                            let event = if velocity == 0 {
                                TrackEvent::NoteOff { channel, key }
                            } else {
                                TrackEvent::NoteOn {
                                    channel,
                                    key,
                                    velocity,
                                }
                            };
                            events.push((tick, sequence, event));
                            sequence += 1;
                        }
                        0xa0 | 0xb0 | 0xe0 => {
                            cursor.data_byte()?;
                            cursor.data_byte()?;
                        }
                        0xc0 => {
                            let program = cursor.data_byte()?;
                            events.push((tick, sequence, TrackEvent::Program { channel, program }));
                            sequence += 1;
                        }
                        0xd0 => {
                            cursor.data_byte()?;
                        }
                        _ => unreachable!("status bytes 0x80..=0xef are exhaustively matched"),
                    }
                }
            }
        }
        position = body_end;
    }

    // Stable order: tick, then original encounter order.
    events.sort_by_key(|&(tick, sequence, _)| (tick, sequence));

    // Tempo map segments as (tick, us_at_tick, us_per_quarter).
    let mut tempo_segments: Vec<(u64, u64, u32)> = vec![(0, 0, 500_000)];
    for &(tick, _, event) in &events {
        if let TrackEvent::Tempo { us_per_quarter } = event {
            let &(last_tick, last_us, last_tempo) = tempo_segments
                .last()
                .expect("seeded with the default tempo");
            let us = last_us + (tick - last_tick) * u64::from(last_tempo) / u64::from(division);
            tempo_segments.push((tick, us, us_per_quarter));
        }
    }
    let tick_to_us = |tick: u64| -> u64 {
        let &(segment_tick, segment_us, tempo) = tempo_segments
            .iter()
            .rev()
            .find(|&&(segment_tick, _, _)| segment_tick <= tick)
            .expect("segment 0 covers tick 0");
        segment_us + (tick - segment_tick) * u64::from(tempo) / u64::from(division)
    };

    let mut programs = [0u8; 16];
    let mut open: Vec<(u8, u8, u64, u8, u8)> = Vec::new(); // channel, key, start_tick, velocity, program
    let mut notes: Vec<MidiNote> = Vec::new();
    let close = |open: &mut Vec<(u8, u8, u64, u8, u8)>,
                 notes: &mut Vec<MidiNote>,
                 channel: u8,
                 key: u8,
                 end_tick: u64| {
        if let Some(index) = open
            .iter()
            .rposition(|&(open_channel, open_key, ..)| open_channel == channel && open_key == key)
        {
            let (_, _, start_tick, velocity, program) = open.remove(index);
            let start_us = tick_to_us(start_tick);
            let end_us = tick_to_us(end_tick.max(start_tick));
            notes.push(MidiNote {
                start_us,
                duration_us: end_us - start_us,
                channel,
                key,
                velocity,
                program,
            });
        }
    };
    let mut max_tick = 0u64;
    for &(tick, _, event) in &events {
        max_tick = max_tick.max(tick);
        match event {
            TrackEvent::Program { channel, program } => programs[channel as usize] = program,
            TrackEvent::NoteOn {
                channel,
                key,
                velocity,
            } => open.push((channel, key, tick, velocity, programs[channel as usize])),
            TrackEvent::NoteOff { channel, key } => {
                close(&mut open, &mut notes, channel, key, tick)
            }
            TrackEvent::Tempo { .. } => {}
        }
    }
    // Close anything left hanging at the end of the song.
    while let Some(&(channel, key, ..)) = open.first() {
        close(&mut open, &mut notes, channel, key, max_tick);
    }
    notes.sort_by_key(|note| (note.start_us, note.channel, note.key));
    let duration_us = notes
        .iter()
        .map(|note| note.start_us + note.duration_us)
        .max()
        .unwrap_or(0);
    Ok(MidiSong {
        format,
        track_count,
        ticks_per_quarter: division,
        notes,
        duration_us,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(format: u16, tracks: u16, division: u16) -> Vec<u8> {
        let mut bytes = b"MThd".to_vec();
        bytes.extend_from_slice(&6u32.to_be_bytes());
        bytes.extend_from_slice(&format.to_be_bytes());
        bytes.extend_from_slice(&tracks.to_be_bytes());
        bytes.extend_from_slice(&division.to_be_bytes());
        bytes
    }

    fn track(events: &[u8]) -> Vec<u8> {
        let mut bytes = b"MTrk".to_vec();
        bytes.extend_from_slice(&(events.len() as u32).to_be_bytes());
        bytes.extend_from_slice(events);
        bytes
    }

    #[test]
    fn a_minimal_song_recovers_exact_microsecond_timing() {
        // 120 BPM tempo, program 42, one quarter note on channel 0.
        let mut events = vec![
            0x00, 0xff, 0x51, 0x03, 0x07, 0xa1, 0x20, // tempo 500000
            0x00, 0xc0, 42, // program 42
            0x00, 0x90, 60, 100, // note on
        ];
        events.extend_from_slice(&[0x83, 0x60, 0x80, 60, 0]); // delta VLQ 480 ticks
        events.extend_from_slice(&[0x00, 0xff, 0x2f, 0x00]);
        let mut bytes = header(0, 1, 480);
        bytes.extend_from_slice(&track(&events));
        let song = parse(&bytes).unwrap();
        assert_eq!(song.notes.len(), 1);
        let note = &song.notes[0];
        assert_eq!(note.key, 60);
        assert_eq!(note.velocity, 100);
        assert_eq!(note.program, 42);
        assert_eq!(note.start_us, 0);
        assert_eq!(note.duration_us, 500_000);
        assert_eq!(song.duration_us, 500_000);
    }

    #[test]
    fn running_status_and_velocity_zero_note_off_work() {
        let events = vec![
            0x00, 0x90, 60, 100, // note on (explicit status)
            0x10, 62, 100, // running status: second note on
            0x10, 60, 0, // running status: velocity-0 off
            0x10, 62, 0, // running status: velocity-0 off
            0x00, 0xff, 0x2f, 0x00,
        ];
        let mut bytes = header(0, 1, 96);
        bytes.extend_from_slice(&track(&events));
        let song = parse(&bytes).unwrap();
        assert_eq!(song.notes.len(), 2);
        assert!(song.notes.iter().all(|note| note.duration_us > 0));
    }

    #[test]
    fn malformed_input_is_rejected_without_panicking() {
        assert_eq!(parse(b""), Err(MidiError::BadHeader));
        assert_eq!(parse(b"RIFFxxxx"), Err(MidiError::BadHeader));
        assert_eq!(
            parse(&header(2, 0, 96)),
            Err(MidiError::UnsupportedFormat(2))
        );
        assert_eq!(parse(&header(1, 0, 0x8000)), Err(MidiError::SmpteDivision));
        // Declared track missing entirely.
        assert_eq!(
            parse(&header(1, 1, 96)),
            Err(MidiError::BadTrackChunk { track: 0 })
        );
        // Track chunk length overruns the blob.
        let mut overrun = header(1, 1, 96);
        overrun.extend_from_slice(b"MTrk");
        overrun.extend_from_slice(&100u32.to_be_bytes());
        overrun.push(0x00);
        assert_eq!(parse(&overrun), Err(MidiError::BadTrackChunk { track: 0 }));
        // Event truncated inside the chunk.
        let mut truncated = header(1, 1, 96);
        truncated.extend_from_slice(&track(&[0x00, 0x90, 60]));
        assert_eq!(parse(&truncated), Err(MidiError::Truncated { track: 0 }));
        // A data byte with the status bit set.
        let mut bad_data = header(1, 1, 96);
        bad_data.extend_from_slice(&track(&[0x00, 0x90, 0x90, 0x40]));
        assert!(matches!(
            parse(&bad_data),
            Err(MidiError::BadEvent { track: 0, .. })
        ));
        // Running status with nothing to run from.
        let mut orphan = header(1, 1, 96);
        orphan.extend_from_slice(&track(&[0x00, 0x40, 0x40]));
        assert!(matches!(
            parse(&orphan),
            Err(MidiError::BadEvent { track: 0, .. })
        ));
    }
}
