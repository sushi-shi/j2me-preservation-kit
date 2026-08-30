//! Versioned filesystem persistence for `j2me_me::RmsRuntime`.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use j2me_me::{RecordStoreSnapshot, RmsRuntime, RmsSnapshot};

use crate::PlatformError;

const MAGIC: &[u8; 8] = b"J2RMS\x01\0\0";
const FILE_NAME: &str = "rms-v1.bin";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn push_u32(output: &mut Vec<u8>, value: usize, label: &str) -> Result<(), PlatformError> {
    let value = u32::try_from(value)
        .map_err(|_| PlatformError::Config(format!("{label} does not fit the RMS format")))?;
    output.extend_from_slice(&value.to_be_bytes());
    Ok(())
}

/// Encode a deterministic, versioned snapshot. This is a host storage format,
/// not a game's save schema and not a claim about a handset's private RMS files.
pub fn encode_rms_snapshot(snapshot: &RmsSnapshot) -> Result<Vec<u8>, PlatformError> {
    RmsRuntime::from_snapshot(snapshot.clone())
        .map_err(|error| PlatformError::Config(error.to_string()))?;
    let mut output = MAGIC.to_vec();
    push_u32(&mut output, snapshot.stores.len(), "store count")?;
    for (name, store) in &snapshot.stores {
        push_u32(&mut output, name.len(), "store-name length")?;
        output.extend_from_slice(name.as_bytes());
        output.extend_from_slice(&store.next_id.to_be_bytes());
        push_u32(&mut output, store.records.len(), "record count")?;
        for (id, record) in &store.records {
            output.extend_from_slice(&id.to_be_bytes());
            push_u32(&mut output, record.len(), "record length")?;
            output.extend(record.iter().map(|byte| *byte as u8));
        }
    }
    Ok(output)
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, length: usize, label: &str) -> Result<&'a [u8], PlatformError> {
        let end = self
            .cursor
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| PlatformError::CorruptRms(format!("truncated {label}")))?;
        let value = &self.bytes[self.cursor..end];
        self.cursor = end;
        Ok(value)
    }

    fn u32(&mut self, label: &str) -> Result<u32, PlatformError> {
        let bytes: [u8; 4] = self.take(4, label)?.try_into().expect("four bytes");
        Ok(u32::from_be_bytes(bytes))
    }

    fn i32(&mut self, label: &str) -> Result<i32, PlatformError> {
        let bytes: [u8; 4] = self.take(4, label)?.try_into().expect("four bytes");
        Ok(i32::from_be_bytes(bytes))
    }
}

/// Decode and structurally validate a host RMS snapshot.
pub fn decode_rms_snapshot(bytes: &[u8]) -> Result<RmsSnapshot, PlatformError> {
    let mut reader = Reader { bytes, cursor: 0 };
    if reader.take(MAGIC.len(), "header")? != MAGIC {
        return Err(PlatformError::CorruptRms("bad magic/version".to_owned()));
    }
    let store_count = reader.u32("store count")? as usize;
    if store_count > bytes.len().saturating_sub(reader.cursor) / 12 {
        return Err(PlatformError::CorruptRms(
            "store count exceeds the remaining payload".to_owned(),
        ));
    }
    let mut stores = BTreeMap::new();
    for _ in 0..store_count {
        let name_length = reader.u32("store-name length")? as usize;
        let name = std::str::from_utf8(reader.take(name_length, "store name")?)
            .map_err(|_| PlatformError::CorruptRms("store name is not UTF-8".to_owned()))?
            .to_owned();
        let next_id = reader.i32("next record id")?;
        let record_count = reader.u32("record count")? as usize;
        if record_count > bytes.len().saturating_sub(reader.cursor) / 8 {
            return Err(PlatformError::CorruptRms(format!(
                "record count for {name:?} exceeds the remaining payload"
            )));
        }
        let mut records = BTreeMap::new();
        for _ in 0..record_count {
            let id = reader.i32("record id")?;
            let length = reader.u32("record length")? as usize;
            let record = reader
                .take(length, "record payload")?
                .iter()
                .map(|byte| *byte as i8)
                .collect();
            if records.insert(id, record).is_some() {
                return Err(PlatformError::CorruptRms(format!(
                    "duplicate record id {id} in {name:?}"
                )));
            }
        }
        if stores
            .insert(name.clone(), RecordStoreSnapshot { next_id, records })
            .is_some()
        {
            return Err(PlatformError::CorruptRms(format!(
                "duplicate store {name:?}"
            )));
        }
    }
    if reader.cursor != bytes.len() {
        return Err(PlatformError::CorruptRms("trailing bytes".to_owned()));
    }
    let snapshot = RmsSnapshot { stores };
    RmsRuntime::from_snapshot(snapshot.clone())
        .map_err(|error| PlatformError::CorruptRms(error.to_string()))?;
    Ok(snapshot)
}

/// A host-owned `RmsRuntime` loaded from and flushed to one filesystem file.
/// Mutation is explicit through `runtime_mut`; callers flush at their chosen
/// durability boundary (normally after a successful game save or shutdown).
#[derive(Debug)]
pub struct PersistentRms {
    path: PathBuf,
    runtime: RmsRuntime,
}

impl PersistentRms {
    pub fn open(directory: impl AsRef<Path>) -> Result<Self, PlatformError> {
        Self::open_with_capacity(directory, None)
    }

    pub fn open_for_profile(
        directory: impl AsRef<Path>,
        profile: &j2me_device::RmsFragment,
    ) -> Result<Self, PlatformError> {
        Self::open_with_capacity(directory, profile.capacity_bytes)
    }

    pub fn open_with_capacity(
        directory: impl AsRef<Path>,
        capacity_bytes: Option<u64>,
    ) -> Result<Self, PlatformError> {
        std::fs::create_dir_all(directory.as_ref())?;
        let path = directory.as_ref().join(FILE_NAME);
        let runtime = match std::fs::read(&path) {
            Ok(bytes) => RmsRuntime::from_snapshot_with_capacity(
                decode_rms_snapshot(&bytes)?,
                capacity_bytes,
            )
            .map_err(|error| PlatformError::CorruptRms(error.to_string()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                RmsRuntime::with_capacity(capacity_bytes)
            }
            Err(error) => return Err(error.into()),
        };
        Ok(Self { path, runtime })
    }

    pub fn runtime(&self) -> &RmsRuntime {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut RmsRuntime {
        &mut self.runtime
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write a complete snapshot to a sibling temporary file, sync it, then
    /// atomically replace the previous snapshot on platforms where rename-over
    /// an existing file is atomic.
    pub fn flush(&self) -> Result<(), PlatformError> {
        let bytes = encode_rms_snapshot(&self.runtime.snapshot())?;
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = self
            .path
            .with_extension(format!("tmp-{}-{sequence}", std::process::id()));
        let result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            #[cfg(target_os = "windows")]
            if self.path.exists() {
                // Windows rename does not replace an existing destination.
                // The temporary file is already complete and synced, but this
                // fallback cannot provide Unix rename-over atomicity.
                std::fs::remove_file(&self.path)?;
            }
            std::fs::rename(&temporary, &self.path)?;
            Ok::<_, std::io::Error>(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result.map_err(PlatformError::Io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_directory(label: &str) -> PathBuf {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "j2me-platform-{label}-{}-{sequence}",
            std::process::id()
        ))
    }

    #[test]
    fn snapshot_round_trip_preserves_ids_names_and_signed_bytes() {
        let mut runtime = RmsRuntime::new();
        runtime.open("settings", true).unwrap();
        runtime
            .get_mut("settings")
            .unwrap()
            .add_record(&[-128, -1, 0, 127], 0, 4)
            .unwrap();
        let snapshot = runtime.snapshot();
        let encoded = encode_rms_snapshot(&snapshot).unwrap();
        assert_eq!(decode_rms_snapshot(&encoded).unwrap(), snapshot);

        let mut truncated = encoded;
        truncated.pop();
        assert!(decode_rms_snapshot(&truncated).is_err());

        let mut impossible_count = MAGIC.to_vec();
        impossible_count.extend_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode_rms_snapshot(&impossible_count).is_err());
    }

    #[test]
    fn filesystem_adapter_reopens_the_same_runtime() {
        let directory = temporary_directory("rms");
        {
            let mut persistent = PersistentRms::open(&directory).unwrap();
            persistent.runtime_mut().open("slot", true).unwrap();
            persistent
                .runtime_mut()
                .get_mut("slot")
                .unwrap()
                .add_record(&[4, 5, 6], 0, 3)
                .unwrap();
            persistent.flush().unwrap();
        }
        let reopened = PersistentRms::open(&directory).unwrap();
        assert_eq!(
            reopened
                .runtime()
                .get("slot")
                .unwrap()
                .get_record(1)
                .unwrap(),
            vec![4, 5, 6]
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
