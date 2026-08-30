//! Bounded parser and deterministic MIDI approximation for Yamaha SMAF/MMF.
//!
//! The parser supports Handy Phone Standard score tracks and preserves
//! unsupported root/track chunks and raw score
//! subchunks so future Mobile Standard work does not require reparsing the
//! original archives.

use crate::writer::Writer;
use crate::{Error, Result};

pub const APPROXIMATION_NOTICE: &str =
    "Approximate SMAF transcription: timing/events are recovered; Yamaha MA synthesis is not emulated.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawChunk {
    pub id: [u8; 4],
    pub offset: usize,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentTag {
    pub tag: String,
    pub value: String,
    pub raw_value: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentsInfo {
    /// Offset of the `CNTI` chunk header, so a writer can restore the original
    /// chunk order without assuming `CNTI` came first.
    pub offset: usize,
    pub contents_class: u8,
    pub contents_type: u8,
    pub code_type: u8,
    pub copy_status: u8,
    pub copy_count: u8,
    pub option_data: Vec<u8>,
    pub tags: Vec<ContentTag>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupMessage {
    SystemExclusive(Vec<u8>),
    Unknown(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Controller {
    Modulation,
    Volume,
    Pan,
    Expression,
}

impl Controller {
    fn midi_number(self) -> u8 {
        match self {
            Self::Modulation => 1,
            Self::Volume => 7,
            Self::Pan => 10,
            Self::Expression => 11,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    Note {
        channel: u8,
        octave: u8,
        note: u8,
        pitch: u8,
        gate: u32,
    },
    ProgramChange {
        channel: u8,
        program: u8,
    },
    BankSelect {
        channel: u8,
        bank: u8,
    },
    OctaveShift {
        channel: u8,
        shift: u8,
    },
    ControlChange {
        channel: u8,
        controller: Controller,
        value: u8,
        /// Selector nibble when the stream used a one-byte compact form.
        /// `None` means the explicit two-byte control form. Modulation and
        /// expression both have a compact table, so the semantic event alone
        /// does not say which encoding the file used.
        compact_selector: Option<u8>,
    },
    PitchBend {
        channel: u8,
        value: u16,
        /// Selector nibble when the stream used the one-byte compact form.
        compact_selector: Option<u8>,
    },
    Meta {
        kind: u8,
        data: Vec<u8>,
    },
    SystemExclusive(Vec<u8>),
    Nop,
    EndOfSequence,
    ReservedShort {
        channel: u8,
        event: u8,
        data: u8,
    },
    UnknownControl {
        channel: u8,
        selector: u8,
        value: u8,
    },
    Unknown(Vec<u8>),
}

impl EventKind {
    pub fn channel(&self) -> Option<u8> {
        match self {
            Self::Note { channel, .. }
            | Self::ProgramChange { channel, .. }
            | Self::BankSelect { channel, .. }
            | Self::OctaveShift { channel, .. }
            | Self::ControlChange { channel, .. }
            | Self::PitchBend { channel, .. }
            | Self::ReservedShort { channel, .. }
            | Self::UnknownControl { channel, .. } => Some(*channel),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// Relative SMAF duration units before this event.
    pub delta: u32,
    pub offset: usize,
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoreTrack {
    pub number: u8,
    pub offset: usize,
    pub format_type: u8,
    pub sequence_type: u8,
    pub duration_time_base_code: u8,
    pub duration_time_base_ms: Option<u16>,
    pub gate_time_base_code: u8,
    pub gate_time_base_ms: Option<u16>,
    pub channel_status: Vec<u8>,
    pub subchunks: Vec<RawChunk>,
    pub setup_messages: Vec<SetupMessage>,
    pub events: Vec<Event>,
}

impl ScoreTrack {
    pub fn note_count(&self) -> usize {
        self.events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::Note { .. }))
            .count()
    }

    pub fn duration_ms(&self) -> Option<u64> {
        let duration_base = u64::from(self.duration_time_base_ms?);
        let gate_base = u64::from(self.gate_time_base_ms?);
        let mut current = 0_u64;
        let mut end = 0_u64;
        for event in &self.events {
            current = current.saturating_add(u64::from(event.delta) * duration_base);
            end = end.max(current);
            if let EventKind::Note { gate, .. } = event.kind {
                end = end.max(current.saturating_add(u64::from(gate) * gate_base));
            }
        }
        Some(end)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmafFile {
    /// `MMMD` payload length, including the stored two-byte CRC.
    pub declared_size: u32,
    pub stored_crc16: u16,
    pub contents: Option<ContentsInfo>,
    pub tracks: Vec<ScoreTrack>,
    pub unknown_chunks: Vec<RawChunk>,
    /// Carrier-specific bytes after the declared `MMMD` payload.
    pub trailer: Vec<u8>,
}

impl SmafFile {
    pub fn event_count(&self) -> usize {
        self.tracks.iter().map(|track| track.events.len()).sum()
    }

    pub fn note_count(&self) -> usize {
        self.tracks.iter().map(ScoreTrack::note_count).sum()
    }

    pub fn duration_ms(&self) -> Option<u64> {
        self.tracks.iter().filter_map(ScoreTrack::duration_ms).max()
    }

    pub fn to_approximate_smf(&self) -> Result<Vec<u8>> {
        to_approximate_smf(self)
    }
}

struct Reader<'a> {
    data: &'a [u8],
    position: usize,
    base: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8], base: usize) -> Self {
        Self {
            data,
            position: 0,
            base,
        }
    }

    fn offset(&self) -> usize {
        self.base + self.position
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.position)
    }

    fn is_empty(&self) -> bool {
        self.position == self.data.len()
    }

    fn bytes(&mut self, length: usize) -> Result<&'a [u8]> {
        if self.remaining() < length {
            return Err(Error::UnexpectedEof {
                offset: self.offset(),
                needed: length,
                remaining: self.remaining(),
            });
        }
        let start = self.position;
        self.position += length;
        Ok(&self.data[start..self.position])
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    fn chunk(&mut self) -> Result<RawChunk> {
        let offset = self.offset();
        let id = self.bytes(4)?.try_into().unwrap();
        let length_offset = self.offset();
        let length = usize::try_from(self.u32()?).map_err(|_| Error::InvalidValue {
            offset: length_offset,
            field: "SMAF chunk length",
            value: u64::MAX,
        })?;
        let data = self.bytes(length)?.to_vec();
        Ok(RawChunk { id, offset, data })
    }
}

pub fn time_base_ms(code: u8) -> Option<u16> {
    match code {
        0x00 => Some(1),
        0x01 => Some(2),
        0x02 => Some(4),
        0x03 => Some(5),
        0x10 => Some(10),
        0x11 => Some(20),
        0x12 => Some(40),
        0x13 => Some(50),
        _ => None,
    }
}

fn latin1(data: &[u8]) -> String {
    data.iter().map(|byte| char::from(*byte)).collect()
}

fn decode_text(data: &[u8], code_type: u8) -> String {
    if code_type == 0x23 {
        String::from_utf8_lossy(data).into_owned()
    } else {
        latin1(data)
    }
}

fn content_tags(option: &[u8], code_type: u8) -> Vec<ContentTag> {
    let mut fields = Vec::<Vec<u8>>::new();
    let mut field = Vec::new();
    let mut escaped = false;
    for byte in option {
        if escaped {
            field.push(*byte);
            escaped = false;
        } else if *byte == b'\\' {
            escaped = true;
        } else if *byte == b',' {
            fields.push(std::mem::take(&mut field));
        } else {
            field.push(*byte);
        }
    }
    if !field.is_empty() {
        fields.push(field);
    }
    fields
        .into_iter()
        .filter_map(|field| {
            let separator = field.iter().position(|byte| *byte == b':')?;
            let raw_value = field[separator + 1..].to_vec();
            Some(ContentTag {
                tag: latin1(&field[..separator]),
                value: decode_text(&raw_value, code_type),
                raw_value,
            })
        })
        .collect()
}

fn parse_contents(chunk: &RawChunk) -> Result<ContentsInfo> {
    let mut reader = Reader::new(&chunk.data, chunk.offset + 8);
    let contents_class = reader.u8()?;
    let contents_type = reader.u8()?;
    let code_type = reader.u8()?;
    let copy_status = reader.u8()?;
    let copy_count = reader.u8()?;
    let option_data = reader.bytes(reader.remaining())?.to_vec();
    let tags = content_tags(&option_data, code_type);
    Ok(ContentsInfo {
        offset: chunk.offset,
        contents_class,
        contents_type,
        code_type,
        copy_status,
        copy_count,
        option_data,
        tags,
    })
}

fn parse_setup(data: &[u8], base: usize) -> Result<Vec<SetupMessage>> {
    let mut reader = Reader::new(data, base);
    let mut messages = Vec::new();
    while !reader.is_empty() {
        let first = reader.u8()?;
        if first != 0xff {
            messages.push(SetupMessage::Unknown(vec![first]));
            continue;
        }
        let second = reader.u8()?;
        if second != 0xf0 {
            messages.push(SetupMessage::Unknown(vec![first, second]));
            continue;
        }
        let length = usize::from(reader.u8()?);
        messages.push(SetupMessage::SystemExclusive(
            reader.bytes(length)?.to_vec(),
        ));
    }
    Ok(messages)
}

/// Handy Phone Standard stores a duration in one byte below 128, or two bytes
/// where the high seven bits encode `(value + 1) * 128`.
fn handy_duration(reader: &mut Reader<'_>) -> Result<u32> {
    let first = reader.u8()?;
    if first & 0x80 == 0 {
        Ok(u32::from(first))
    } else {
        let second = reader.u8()?;
        Ok(((u32::from(first & 0x7f) + 1) << 7) | u32::from(second))
    }
}

fn parse_sequence(data: &[u8], base: usize) -> Result<Vec<Event>> {
    const MODULATION: [Option<u8>; 16] = [
        None,
        Some(0),
        Some(8),
        Some(16),
        Some(24),
        Some(32),
        Some(40),
        Some(48),
        Some(56),
        Some(64),
        Some(72),
        Some(80),
        Some(96),
        Some(112),
        Some(127),
        None,
    ];

    let mut reader = Reader::new(data, base);
    let mut events = Vec::new();
    while !reader.is_empty() {
        let offset = reader.offset();
        let delta = handy_duration(&mut reader)?;
        let first = reader.u8()?;
        let kind = if first == 0xff {
            let second = reader.u8()?;
            match second {
                0x2f | 0x51 | 0x58 => {
                    let length = usize::from(reader.u8()?);
                    EventKind::Meta {
                        kind: second,
                        data: reader.bytes(length)?.to_vec(),
                    }
                }
                0xf0 => {
                    let length = usize::from(reader.u8()?);
                    EventKind::SystemExclusive(reader.bytes(length)?.to_vec())
                }
                0x00 => EventKind::Nop,
                _ => {
                    return Err(Error::InvalidValue {
                        offset: reader.offset() - 1,
                        field: "Handy Phone 0xff event",
                        value: u64::from(second),
                    });
                }
            }
        } else if first != 0 {
            let gate = handy_duration(&mut reader)?;
            let channel = (first & 0xc0) >> 6;
            let octave = (first & 0x30) >> 4;
            let note = first & 0x0f;
            EventKind::Note {
                channel,
                octave,
                note,
                pitch: note.saturating_add(octave * 12),
                gate,
            }
        } else {
            let second = reader.u8()?;
            if second == 0 {
                let third = reader.u8()?;
                if third == 0 {
                    EventKind::EndOfSequence
                } else {
                    EventKind::Unknown(vec![first, second, third])
                }
            } else {
                let channel = (second & 0xc0) >> 6;
                let event = (second & 0x30) >> 4;
                let selector = second & 0x0f;
                match event {
                    3 => {
                        let value = reader.u8()?;
                        match selector {
                            0 => EventKind::ProgramChange {
                                channel,
                                program: value,
                            },
                            1 => EventKind::BankSelect {
                                channel,
                                bank: value,
                            },
                            2 => EventKind::OctaveShift {
                                channel,
                                shift: value,
                            },
                            3 => EventKind::ControlChange {
                                channel,
                                controller: Controller::Modulation,
                                value,
                                compact_selector: None,
                            },
                            4 => EventKind::PitchBend {
                                channel,
                                value: u16::from(value) << 7,
                                compact_selector: None,
                            },
                            7 => EventKind::ControlChange {
                                channel,
                                controller: Controller::Volume,
                                value,
                                compact_selector: None,
                            },
                            0x0a => EventKind::ControlChange {
                                channel,
                                controller: Controller::Pan,
                                value,
                                compact_selector: None,
                            },
                            0x0b => EventKind::ControlChange {
                                channel,
                                controller: Controller::Expression,
                                value,
                                compact_selector: None,
                            },
                            _ => EventKind::UnknownControl {
                                channel,
                                selector,
                                value,
                            },
                        }
                    }
                    2 => MODULATION[usize::from(selector)].map_or(
                        EventKind::ReservedShort {
                            channel,
                            event,
                            data: selector,
                        },
                        |value| EventKind::ControlChange {
                            channel,
                            controller: Controller::Modulation,
                            value,
                            compact_selector: Some(selector),
                        },
                    ),
                    1 => EventKind::PitchBend {
                        channel,
                        value: (u16::from(selector) * 8) << 7,
                        compact_selector: Some(selector),
                    },
                    0 => EventKind::ControlChange {
                        channel,
                        controller: Controller::Expression,
                        value: if selector == 1 {
                            0
                        } else {
                            selector.saturating_mul(8).saturating_add(15)
                        },
                        compact_selector: Some(selector),
                    },
                    _ => unreachable!(),
                }
            }
        };
        events.push(Event {
            delta,
            offset,
            kind,
        });
    }
    Ok(events)
}

fn status_length(format_type: u8, offset: usize) -> Result<usize> {
    match format_type {
        0 => Ok(2),
        1 | 2 => Ok(16),
        3 => Ok(32),
        _ => Err(Error::InvalidValue {
            offset,
            field: "SMAF score format type",
            value: u64::from(format_type),
        }),
    }
}

fn parse_track(chunk: &RawChunk) -> Result<ScoreTrack> {
    let mut reader = Reader::new(&chunk.data, chunk.offset + 8);
    let format_offset = reader.offset();
    let format_type = reader.u8()?;
    let sequence_type = reader.u8()?;
    let duration_time_base_code = reader.u8()?;
    let gate_time_base_code = reader.u8()?;
    let channel_status = reader
        .bytes(status_length(format_type, format_offset)?)?
        .to_vec();
    let mut subchunks = Vec::new();
    let mut setup_messages = Vec::new();
    let mut events = Vec::new();
    while !reader.is_empty() {
        let subchunk = reader.chunk()?;
        if format_type == 0 && &subchunk.id == b"Mtsu" {
            setup_messages.extend(parse_setup(&subchunk.data, subchunk.offset + 8)?);
        } else if format_type == 0 && &subchunk.id == b"Mtsq" {
            events.extend(parse_sequence(&subchunk.data, subchunk.offset + 8)?);
        }
        subchunks.push(subchunk);
    }
    Ok(ScoreTrack {
        number: chunk.id[3],
        offset: chunk.offset,
        format_type,
        sequence_type,
        duration_time_base_code,
        duration_time_base_ms: time_base_ms(duration_time_base_code),
        gate_time_base_code,
        gate_time_base_ms: time_base_ms(gate_time_base_code),
        channel_status,
        subchunks,
        setup_messages,
        events,
    })
}

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xffff_u16;
    for byte in data {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    !crc
}

pub fn parse(data: &[u8]) -> Result<SmafFile> {
    let mut reader = Reader::new(data, 0);
    if reader.bytes(4)? != b"MMMD" {
        return Err(Error::InvalidMagic { format: "SMAF" });
    }
    let declared_offset = reader.offset();
    let declared_size = reader.u32()?;
    let declared_size_usize = usize::try_from(declared_size).map_err(|_| Error::InvalidValue {
        offset: declared_offset,
        field: "MMMD length",
        value: u64::from(declared_size),
    })?;
    if declared_size_usize < 2 {
        return Err(Error::InvalidValue {
            offset: declared_offset,
            field: "MMMD length",
            value: u64::from(declared_size),
        });
    }
    let payload = reader.bytes(declared_size_usize)?;
    let checksum_offset = 8 + declared_size_usize - 2;
    let stored_crc16 = u16::from_be_bytes(payload[payload.len() - 2..].try_into().unwrap());
    let computed = crc16(&data[..checksum_offset]);
    if stored_crc16 != computed {
        return Err(Error::ChecksumMismatch {
            offset: checksum_offset,
            stored: u32::from(stored_crc16),
            computed: u32::from(computed),
        });
    }
    let trailer = reader.bytes(reader.remaining())?.to_vec();
    let mut chunks = Reader::new(&payload[..payload.len() - 2], 8);
    let mut contents = None;
    let mut tracks = Vec::new();
    let mut unknown_chunks = Vec::new();
    while !chunks.is_empty() {
        let chunk = chunks.chunk()?;
        if &chunk.id == b"CNTI" {
            contents = Some(parse_contents(&chunk)?);
        } else if &chunk.id[..3] == b"MTR" {
            tracks.push(parse_track(&chunk)?);
        } else {
            unknown_chunks.push(chunk);
        }
    }
    Ok(SmafFile {
        declared_size,
        stored_crc16,
        contents,
        tracks,
        unknown_chunks,
        trailer,
    })
}

/// Emits a Handy Phone Standard duration: one byte below 128, otherwise two
/// where the high seven bits carry `(value >> 7) - 1`.
///
/// The parser ORs the whole second byte rather than masking it, so a file that
/// set the second byte's high bit would decode to a value this writer cannot
/// re-encode. Such a duration is rejected rather than truncated.
fn write_handy_duration(writer: &mut Writer, value: u32) {
    if value < 128 {
        writer.u8(value as u8);
    } else {
        writer.u8(0x80 | ((((value >> 7) - 1) & 0x7f) as u8));
        writer.u8((value & 0x7f) as u8);
    }
}

fn control_selector(controller: Controller) -> u8 {
    match controller {
        Controller::Modulation => 3,
        Controller::Volume => 7,
        Controller::Pan => 0x0a,
        Controller::Expression => 0x0b,
    }
}

/// Compact-form event nibble for the controllers that have one.
fn compact_event(controller: Controller) -> u8 {
    match controller {
        Controller::Modulation => 2,
        _ => 0,
    }
}

fn write_event(writer: &mut Writer, event: &Event) {
    write_handy_duration(writer, event.delta);
    match &event.kind {
        EventKind::Note {
            channel,
            octave,
            note,
            gate,
            ..
        } => {
            writer.u8((channel << 6) | (octave << 4) | note);
            write_handy_duration(writer, *gate);
        }
        EventKind::Meta { kind, data } => {
            writer.u8(0xff).u8(*kind).u8(data.len() as u8).bytes(data);
        }
        EventKind::SystemExclusive(data) => {
            writer.u8(0xff).u8(0xf0).u8(data.len() as u8).bytes(data);
        }
        EventKind::Nop => {
            writer.u8(0xff).u8(0x00);
        }
        EventKind::EndOfSequence => {
            writer.u8(0).u8(0).u8(0);
        }
        EventKind::Unknown(bytes) => {
            writer.bytes(bytes);
        }
        EventKind::ProgramChange { channel, program } => {
            writer.u8(0).u8((channel << 6) | 0x30).u8(*program);
        }
        EventKind::BankSelect { channel, bank } => {
            writer.u8(0).u8((channel << 6) | 0x31).u8(*bank);
        }
        EventKind::OctaveShift { channel, shift } => {
            writer.u8(0).u8((channel << 6) | 0x32).u8(*shift);
        }
        EventKind::ControlChange {
            channel,
            controller,
            value,
            compact_selector,
        } => match compact_selector {
            Some(selector) => {
                writer
                    .u8(0)
                    .u8((channel << 6) | (compact_event(*controller) << 4) | selector);
            }
            None => {
                writer
                    .u8(0)
                    .u8((channel << 6) | 0x30 | control_selector(*controller))
                    .u8(*value);
            }
        },
        EventKind::PitchBend {
            channel,
            value,
            compact_selector,
        } => match compact_selector {
            Some(selector) => {
                writer.u8(0).u8((channel << 6) | 0x10 | selector);
            }
            None => {
                writer
                    .u8(0)
                    .u8((channel << 6) | 0x34)
                    .u8((value >> 7) as u8);
            }
        },
        EventKind::ReservedShort {
            channel,
            event,
            data,
        } => {
            writer.u8(0).u8((channel << 6) | (event << 4) | data);
        }
        EventKind::UnknownControl {
            channel,
            selector,
            value,
        } => {
            writer.u8(0).u8((channel << 6) | 0x30 | selector).u8(*value);
        }
    }
}

fn write_setup(writer: &mut Writer, messages: &[SetupMessage]) {
    for message in messages {
        match message {
            SetupMessage::SystemExclusive(data) => {
                writer.u8(0xff).u8(0xf0).u8(data.len() as u8).bytes(data);
            }
            SetupMessage::Unknown(bytes) => {
                writer.bytes(bytes);
            }
        }
    }
}

fn write_chunk(writer: &mut Writer, id: &[u8; 4], data: &[u8]) {
    writer.bytes(id).u32(data.len() as u32).bytes(data);
}

fn write_track(track: &ScoreTrack) -> Vec<u8> {
    let mut writer = Writer::new();
    writer
        .u8(track.format_type)
        .u8(track.sequence_type)
        .u8(track.duration_time_base_code)
        .u8(track.gate_time_base_code)
        .bytes(&track.channel_status);
    // The decoded setup and sequence lists are flattened across subchunks, so
    // they can only be re-encoded when the track holds a single one of each.
    let single = |id: &[u8; 4]| {
        track
            .subchunks
            .iter()
            .filter(|chunk| &chunk.id == id)
            .count()
            == 1
    };
    let rebuild_setup = track.format_type == 0 && single(b"Mtsu");
    let rebuild_sequence = track.format_type == 0 && single(b"Mtsq");
    for chunk in &track.subchunks {
        if rebuild_setup && &chunk.id == b"Mtsu" {
            let mut body = Writer::new();
            write_setup(&mut body, &track.setup_messages);
            write_chunk(&mut writer, &chunk.id, &body.into_bytes());
        } else if rebuild_sequence && &chunk.id == b"Mtsq" {
            let mut body = Writer::new();
            for event in &track.events {
                write_event(&mut body, event);
            }
            write_chunk(&mut writer, &chunk.id, &body.into_bytes());
        } else {
            write_chunk(&mut writer, &chunk.id, &chunk.data);
        }
    }
    writer.into_bytes()
}

/// Re-encodes a file. `write(&parse(blob)?)` must equal `blob` byte for byte.
///
/// Chunks are restored to their recorded file order rather than to the
/// `contents`/`tracks`/`unknown_chunks` grouping the parser sorts them into.
/// The CRC is recomputed over the emitted bytes; the parser has already proved
/// it equals `stored_crc16`.
pub fn write(file: &SmafFile) -> Vec<u8> {
    let mut chunks: Vec<(usize, Vec<u8>)> = Vec::new();
    if let Some(contents) = &file.contents {
        let mut body = Writer::new();
        body.u8(contents.contents_class)
            .u8(contents.contents_type)
            .u8(contents.code_type)
            .u8(contents.copy_status)
            .u8(contents.copy_count)
            .bytes(&contents.option_data);
        let mut chunk = Writer::new();
        write_chunk(&mut chunk, b"CNTI", &body.into_bytes());
        chunks.push((contents.offset, chunk.into_bytes()));
    }
    for track in &file.tracks {
        let mut chunk = Writer::new();
        write_chunk(
            &mut chunk,
            &[b'M', b'T', b'R', track.number],
            &write_track(track),
        );
        chunks.push((track.offset, chunk.into_bytes()));
    }
    for unknown in &file.unknown_chunks {
        let mut chunk = Writer::new();
        write_chunk(&mut chunk, &unknown.id, &unknown.data);
        chunks.push((unknown.offset, chunk.into_bytes()));
    }
    chunks.sort_by_key(|(offset, _)| *offset);

    let mut writer = Writer::new();
    writer.bytes(b"MMMD").u32(file.declared_size);
    for (_, chunk) in chunks {
        writer.bytes(&chunk);
    }
    let mut output = writer.into_bytes();
    let checksum = crc16(&output);
    output.extend_from_slice(&checksum.to_be_bytes());
    output.extend_from_slice(&file.trailer);
    output
}

#[derive(Clone)]
struct ScheduledMidiEvent {
    tick: u64,
    priority: u8,
    order: usize,
    bytes: Vec<u8>,
}

fn midi_channel(track_index: usize, channel: u8) -> u8 {
    const MELODIC: [u8; 15] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13, 14, 15];
    MELODIC[(track_index * 4 + usize::from(channel)) % MELODIC.len()]
}

fn initial_percussion(track_index: usize, channel_status: &[u8], channel: u8) -> bool {
    let status = channel_status
        .get(usize::from(channel / 2))
        .copied()
        .unwrap_or_default();
    let nibble = if channel % 2 == 0 {
        status >> 4
    } else {
        status & 0x0f
    };
    match nibble & 0x03 {
        // "No care" channels use the conventional pseudo-MIDI channel 9 as rhythm.
        0 => track_index * 4 + usize::from(channel) == 9,
        // Explicit rhythm channel.
        3 => true,
        // Melody and unused/non-melody modes do not imply a rhythm mapping.
        1 | 2 => false,
        _ => unreachable!(),
    }
}

fn octave_semitones(value: u8) -> i16 {
    match value {
        1..=4 => i16::from(value) * 12,
        0x81..=0x84 => -i16::from(value - 0x80) * 12,
        _ => 0,
    }
}

fn schedule(events: &mut Vec<ScheduledMidiEvent>, tick: u64, priority: u8, bytes: Vec<u8>) {
    let order = events.len();
    events.push(ScheduledMidiEvent {
        tick,
        priority,
        order,
        bytes,
    });
}

fn midi_vlq(value: u32, output: &mut Vec<u8>) {
    let mut buffer = [0_u8; 4];
    let mut index = 3;
    buffer[index] = (value & 0x7f) as u8;
    let mut remaining = value >> 7;
    while remaining != 0 {
        index -= 1;
        buffer[index] = ((remaining & 0x7f) as u8) | 0x80;
        remaining >>= 7;
    }
    output.extend_from_slice(&buffer[index..]);
}

fn midi_track(mut events: Vec<ScheduledMidiEvent>, end_tick: u64) -> Result<Vec<u8>> {
    events.sort_by_key(|event| (event.tick, event.priority, event.order));
    let mut payload = Vec::new();
    let mut previous = 0_u64;
    for event in events {
        let delta = event.tick.saturating_sub(previous);
        if delta > 0x0fff_ffff {
            return Err(Error::InvalidValue {
                offset: 0,
                field: "SMF delta time",
                value: delta,
            });
        }
        midi_vlq(delta as u32, &mut payload);
        payload.extend_from_slice(&event.bytes);
        previous = event.tick;
    }
    let delta = end_tick.saturating_sub(previous);
    if delta > 0x0fff_ffff {
        return Err(Error::InvalidValue {
            offset: 0,
            field: "SMF end delta time",
            value: delta,
        });
    }
    midi_vlq(delta as u32, &mut payload);
    payload.extend_from_slice(&[0xff, 0x2f, 0x00]);
    let length = u32::try_from(payload.len()).map_err(|_| Error::InvalidValue {
        offset: 0,
        field: "SMF track length",
        value: payload.len() as u64,
    })?;
    let mut output = b"MTrk".to_vec();
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(&payload);
    Ok(output)
}

pub fn to_approximate_smf(file: &SmafFile) -> Result<Vec<u8>> {
    let convertible = file
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, track)| track.format_type == 0)
        .collect::<Vec<_>>();
    if convertible.is_empty() {
        return Err(Error::InvalidValue {
            offset: 0,
            field: "convertible SMAF score track count",
            value: 0,
        });
    }
    let track_count = u16::try_from(convertible.len() + 1).map_err(|_| Error::InvalidValue {
        offset: 0,
        field: "SMF track count",
        value: (convertible.len() + 1) as u64,
    })?;
    let mut output = b"MThd\0\0\0\x06\0\x01".to_vec();
    output.extend_from_slice(&track_count.to_be_bytes());
    output.extend_from_slice(&1000_u16.to_be_bytes());

    let mut conductor = Vec::new();
    schedule(
        &mut conductor,
        0,
        0,
        [vec![0xff, 0x03, 18], b"SMAF approximation".to_vec()].concat(),
    );
    schedule(
        &mut conductor,
        0,
        1,
        [
            vec![0xff, 0x01, APPROXIMATION_NOTICE.len() as u8],
            APPROXIMATION_NOTICE.as_bytes().to_vec(),
        ]
        .concat(),
    );
    schedule(
        &mut conductor,
        0,
        2,
        vec![0xff, 0x51, 0x03, 0x0f, 0x42, 0x40],
    );
    output.extend_from_slice(&midi_track(conductor, 0)?);

    for (track_index, track) in convertible {
        let duration_base = u64::from(track.duration_time_base_ms.ok_or(Error::InvalidValue {
            offset: track.offset + 10,
            field: "SMAF duration time base",
            value: u64::from(track.duration_time_base_code),
        })?);
        let gate_base = u64::from(track.gate_time_base_ms.ok_or(Error::InvalidValue {
            offset: track.offset + 11,
            field: "SMAF gate time base",
            value: u64::from(track.gate_time_base_code),
        })?);
        let mut scheduled = Vec::new();
        let name = format!("SMAF MTR{} approximation", track.number);
        schedule(
            &mut scheduled,
            0,
            0,
            [vec![0xff, 0x03, name.len() as u8], name.into_bytes()].concat(),
        );
        for setup in &track.setup_messages {
            if let SetupMessage::SystemExclusive(data) = setup {
                let mut message = vec![0xf0];
                midi_vlq(data.len() as u32, &mut message);
                message.extend_from_slice(data);
                schedule(&mut scheduled, 0, 1, message);
            }
        }
        let mut current = 0_u64;
        let mut end = 0_u64;
        let mut programs = [0_u8; 4];
        let mut octave = [0_u8; 4];
        let mut percussion: [bool; 4] = std::array::from_fn(|channel| {
            initial_percussion(track_index, &track.channel_status, channel as u8)
        });
        let mut velocity = [127_u8; 4];
        for event in &track.events {
            current = current.saturating_add(u64::from(event.delta) * duration_base);
            end = end.max(current);
            match event.kind {
                EventKind::Note {
                    channel,
                    pitch,
                    gate,
                    ..
                } => {
                    // Handy Phone Standard treats a zero gate as a non-sounding note.
                    if gate == 0 {
                        continue;
                    }
                    let state = usize::from(channel);
                    let midi_channel = if percussion[state] {
                        9
                    } else {
                        midi_channel(track_index, channel)
                    };
                    let midi_pitch = if percussion[state] {
                        programs[state]
                    } else {
                        (i16::from(pitch) + 36 + octave_semitones(octave[state])).clamp(0, 127)
                            as u8
                    };
                    schedule(
                        &mut scheduled,
                        current,
                        3,
                        vec![0x90 | midi_channel, midi_pitch, velocity[state]],
                    );
                    let note_end = current.saturating_add(u64::from(gate) * gate_base);
                    schedule(
                        &mut scheduled,
                        note_end,
                        0,
                        vec![0x80 | midi_channel, midi_pitch, 0],
                    );
                    end = end.max(note_end);
                }
                EventKind::ProgramChange { channel, program } => {
                    let state = usize::from(channel);
                    programs[state] = program & 0x7f;
                    if !percussion[state] {
                        schedule(
                            &mut scheduled,
                            current,
                            2,
                            vec![0xc0 | midi_channel(track_index, channel), programs[state]],
                        );
                    }
                }
                EventKind::BankSelect { channel, bank } => {
                    percussion[usize::from(channel)] = bank & 0x80 != 0;
                }
                EventKind::OctaveShift { channel, shift } => {
                    octave[usize::from(channel)] = shift;
                }
                EventKind::ControlChange {
                    channel,
                    controller,
                    value,
                    ..
                } => {
                    let state = usize::from(channel);
                    if controller == Controller::Volume && percussion[state] {
                        velocity[state] = value & 0x7f;
                    } else {
                        let midi_channel = if percussion[state] {
                            9
                        } else {
                            midi_channel(track_index, channel)
                        };
                        schedule(
                            &mut scheduled,
                            current,
                            2,
                            vec![0xb0 | midi_channel, controller.midi_number(), value & 0x7f],
                        );
                    }
                }
                EventKind::PitchBend { channel, value, .. } => {
                    let state = usize::from(channel);
                    let midi_channel = if percussion[state] {
                        9
                    } else {
                        midi_channel(track_index, channel)
                    };
                    let value = value.min(0x3fff);
                    schedule(
                        &mut scheduled,
                        current,
                        2,
                        vec![0xe0 | midi_channel, value as u8 & 0x7f, (value >> 7) as u8],
                    );
                }
                EventKind::SystemExclusive(ref data) => {
                    let mut message = vec![0xf0];
                    midi_vlq(data.len() as u32, &mut message);
                    message.extend_from_slice(data);
                    schedule(&mut scheduled, current, 1, message);
                }
                EventKind::Meta {
                    kind: 0x58,
                    ref data,
                } => {
                    let mut message = vec![0xff, 0x58, data.len() as u8];
                    message.extend_from_slice(data);
                    schedule(&mut scheduled, current, 1, message);
                }
                _ => {}
            }
        }
        output.extend_from_slice(&midi_track(scheduled, end)?);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_file() -> Vec<u8> {
        let sequence = [
            0x00, 0x00, 0x30, 0x28, // program 40
            0x00, 0x34, 0x20, // note, gate 32
            0x10, 0x00, 0x00, 0x00, // end after 16 units
        ];
        let mut track = vec![0, 0, 2, 2, 0x80, 0x80];
        track.extend_from_slice(b"Mtsq");
        track.extend_from_slice(&(sequence.len() as u32).to_be_bytes());
        track.extend_from_slice(&sequence);
        let contents = [
            0, 0, 1, 0, 0, b'S', b'T', b':', b'T', b'e', b's', b't', b',',
        ];
        let mut file = b"MMMD\0\0\0\0".to_vec();
        file.extend_from_slice(b"CNTI");
        file.extend_from_slice(&(contents.len() as u32).to_be_bytes());
        file.extend_from_slice(&contents);
        file.extend_from_slice(b"MTR\x01");
        file.extend_from_slice(&(track.len() as u32).to_be_bytes());
        file.extend_from_slice(&track);
        let declared = (file.len() + 2 - 8) as u32;
        file[4..8].copy_from_slice(&declared.to_be_bytes());
        let checksum = crc16(&file);
        file.extend_from_slice(&checksum.to_be_bytes());
        file.extend_from_slice(&[0x1d, 0x0f]);
        file
    }

    #[test]
    fn parses_handset_score_and_metadata() {
        let file = parse(&sample_file()).unwrap();
        assert_eq!(file.trailer, [0x1d, 0x0f]);
        assert_eq!(file.contents.unwrap().tags[0].value, "Test");
        assert_eq!(file.tracks.len(), 1);
        assert_eq!(file.tracks[0].note_count(), 1);
        assert_eq!(file.tracks[0].duration_ms(), Some(128));
    }

    #[test]
    fn rejects_checksum_damage() {
        let mut data = sample_file();
        data[20] ^= 1;
        assert!(matches!(parse(&data), Err(Error::ChecksumMismatch { .. })));
    }

    #[test]
    fn emits_standard_midi_container() {
        let file = parse(&sample_file()).unwrap();
        let midi = file.to_approximate_smf().unwrap();
        assert_eq!(&midi[..4], b"MThd");
        assert_eq!(u16::from_be_bytes(midi[10..12].try_into().unwrap()), 2);
        assert!(midi.windows(4).any(|bytes| bytes == b"MTrk"));
        assert!(midi.windows(3).any(|bytes| bytes == [0x90, 76, 127]));
    }

    #[test]
    fn recovers_default_and_explicit_rhythm_channels() {
        assert!(!initial_percussion(0, &[0x80, 0x80], 1));
        assert!(initial_percussion(2, &[0x80, 0x80], 1));
        assert!(initial_percussion(0, &[0xb0, 0x80], 0));
        assert!(!initial_percussion(2, &[0x90, 0x80], 0));
    }
}
