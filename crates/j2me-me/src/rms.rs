//! `javax.microedition.rms.RecordStore` — a host-owned in-memory model of the
//! MIDP record store the save system persists into.
//!
//! [`RmsRuntime`] is the RMS host seam: the game shell owns one and threads
//! `&mut RmsRuntime` through the transliterated save/load code (the same pattern
//! the media and graphics runtimes use — a single host-owned surface the game
//! drives). It lets the save methods be transliterated faithfully AND
//! round-tripped (write a slot, read it back).
//!
//! The modeled surface is the one a typical save-file wrapper touches:
//! `openRecordStore(name, create)`, `closeRecordStore()`,
//! `deleteRecordStore(name)`, `getNumRecords()`, `getNextRecordID()`,
//! `getRecordSize(id)`, `getRecord(id)`, `addRecord(data, off, len)`. Methods a
//! strict port does not call — `setRecord`/`deleteRecord` — are left unmodeled
//! until a game needs them (no APIs the game never calls). Record IDs are
//! **monotonic** as in MIDP — assigned by `addRecord`, never reused, reset only
//! when the store itself is deleted and recreated (the delete-and-recreate a
//! single-packed-record save wrapper does on close).

use j2me_jvm::JavaError;
use std::collections::BTreeMap;
use std::collections::HashMap;

fn not_found(name: &str) -> JavaError {
    JavaError::RecordStore(format!("RecordStoreNotFoundException: {name}"))
}

fn invalid_id(id: i32) -> JavaError {
    JavaError::RecordStore(format!("InvalidRecordIDException: {id}"))
}

/// One record store: records keyed by their monotonic 1-based id, plus the id
/// the next `addRecord` will assign (MIDP's `getNextRecordID`).
#[derive(Debug, Clone)]
pub struct RecordStore {
    records: BTreeMap<i32, Vec<i8>>,
    next_id: i32,
}

impl Default for RecordStore {
    fn default() -> Self {
        Self {
            records: BTreeMap::new(),
            // MIDP: the first record ever added to a fresh store gets id 1.
            next_id: 1,
        }
    }
}

impl RecordStore {
    /// `getNumRecords()`.
    pub fn num_records(&self) -> i32 {
        self.records.len() as i32
    }

    /// `getNextRecordID()` — the id the next `addRecord` will assign. On a fresh
    /// store this is 1; the game reads `getNextRecordID() - 1` to address the
    /// most recently added record.
    pub fn next_record_id(&self) -> i32 {
        self.next_id
    }

    /// `addRecord(data, offset, numBytes)` — appends a record with the next
    /// monotonic id and returns it. Bounds are checked with Java semantics — a
    /// slice outside the array is `ArrayIndexOutOfBoundsException`, never a panic.
    pub fn add_record(
        &mut self,
        data: &[i8],
        offset: i32,
        num_bytes: i32,
    ) -> Result<i32, JavaError> {
        let rec = slice_checked(data, offset, num_bytes)?.to_vec();
        let id = self.next_id;
        self.records.insert(id, rec);
        self.next_id += 1;
        Ok(id)
    }

    /// `getRecord(id)` — a copy of the record's bytes; a missing id throws
    /// `InvalidRecordIDException`.
    pub fn get_record(&self, id: i32) -> Result<Vec<i8>, JavaError> {
        self.records.get(&id).cloned().ok_or_else(|| invalid_id(id))
    }

    /// `getRecordSize(id)` — the record's length in bytes; a missing id throws.
    pub fn get_record_size(&self, id: i32) -> Result<i32, JavaError> {
        self.records
            .get(&id)
            .map(|r| r.len() as i32)
            .ok_or_else(|| invalid_id(id))
    }
}

/// Java-semantics slice: reject a negative offset/length or a range past the end
/// with `ArrayIndexOutOfBoundsException` rather than panicking (R10).
fn slice_checked(data: &[i8], offset: i32, num_bytes: i32) -> Result<&[i8], JavaError> {
    let len = data.len() as i64;
    let off = offset as i64;
    let n = num_bytes as i64;
    if offset < 0 || num_bytes < 0 || off + n > len {
        return Err(JavaError::ArrayIndexOutOfBounds {
            index: if offset < 0 {
                offset
            } else {
                offset.wrapping_add(num_bytes)
            },
            length: data.len() as i32,
        });
    }
    Ok(&data[offset as usize..(offset + num_bytes) as usize])
}

/// The set of named record stores on the device (the RMS namespace) — the host
/// seam. `RmsRuntime` owns the persistent bytes; a store handle in the
/// transliterated code is its name.
#[derive(Debug, Default, Clone)]
pub struct RmsRuntime {
    stores: HashMap<String, RecordStore>,
}

impl RmsRuntime {
    /// A fresh, empty RMS namespace.
    pub fn new() -> Self {
        Self::default()
    }

    /// `RecordStore.openRecordStore(name, createIfNecessary)`. Returns the store
    /// handle (its name); when the store is absent and `create` is false, throws
    /// `RecordStoreNotFoundException` (the `exists()` / read-mode probe relies on
    /// this).
    pub fn open(&mut self, name: &str, create: bool) -> Result<String, JavaError> {
        if !self.stores.contains_key(name) {
            if create {
                self.stores.insert(name.to_string(), RecordStore::default());
            } else {
                return Err(not_found(name));
            }
        }
        Ok(name.to_string())
    }

    /// `closeRecordStore()` — closes the handle. The in-memory store persists in
    /// the namespace (a real device keeps the record store across a close); this
    /// only validates the store exists, matching MIDP's throw-if-absent.
    pub fn close(&self, name: &str) -> Result<(), JavaError> {
        if self.stores.contains_key(name) {
            Ok(())
        } else {
            Err(not_found(name))
        }
    }

    /// `RecordStore.deleteRecordStore(name)` — removes the store (and its ids)
    /// entirely; throws `RecordStoreNotFoundException` if it does not exist.
    pub fn delete_store(&mut self, name: &str) -> Result<(), JavaError> {
        self.stores
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| not_found(name))
    }

    /// Whether a record store with this name exists (backs an `exists()` probe).
    pub fn contains(&self, name: &str) -> bool {
        self.stores.contains_key(name)
    }

    /// Shared access to an open store (`getNumRecords`/`getRecord`/… dispatch
    /// through here). A missing store is `RecordStoreNotFoundException`.
    pub fn get(&self, name: &str) -> Result<&RecordStore, JavaError> {
        self.stores.get(name).ok_or_else(|| not_found(name))
    }

    /// Mutable access to an open store (`addRecord` dispatches through here).
    pub fn get_mut(&mut self, name: &str) -> Result<&mut RecordStore, JavaError> {
        // Borrow-checker-friendly presence check before the mutable borrow.
        if !self.stores.contains_key(name) {
            return Err(not_found(name));
        }
        Ok(self.stores.get_mut(name).expect("just checked present"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_requires_existing_store_when_not_creating() {
        let mut rms = RmsRuntime::new();
        // Read-mode / exists-probe on an absent store throws.
        assert!(rms.open("save_slot0", false).is_err());
        assert!(!rms.contains("save_slot0"));
        // Create-mode makes it.
        assert_eq!(rms.open("save_slot0", true).unwrap(), "save_slot0");
        assert!(rms.contains("save_slot0"));
        // Now the read-mode open succeeds.
        assert!(rms.open("save_slot0", false).is_ok());
    }

    #[test]
    fn add_then_read_back_round_trips_the_bytes() {
        // The single-packed-record pattern: write one record, read it back via
        // getRecord(getNextRecordID() - 1).
        let mut rms = RmsRuntime::new();
        rms.open("opt", true).unwrap();
        let payload: Vec<i8> = vec![7, -3, 42, 0, 100];
        let store = rms.get_mut("opt").unwrap();
        let id = store.add_record(&payload, 0, payload.len() as i32).unwrap();
        assert_eq!(id, 1, "first record in a fresh store is id 1");

        let store = rms.get("opt").unwrap();
        assert_eq!(store.num_records(), 1);
        assert_eq!(store.next_record_id(), 2);
        // getRecord(getNextRecordID() - 1) addresses the last record.
        let last = store.next_record_id() - 1;
        assert_eq!(store.get_record(last).unwrap(), payload);
        assert_eq!(store.get_record_size(last).unwrap(), payload.len() as i32);
    }

    #[test]
    fn record_ids_are_monotonic_and_survive_a_delete_but_reset_on_recreate() {
        let mut rms = RmsRuntime::new();
        rms.open("s", true).unwrap();
        let s = rms.get_mut("s").unwrap();
        assert_eq!(s.add_record(&[1], 0, 1).unwrap(), 1);
        assert_eq!(s.add_record(&[2], 0, 1).unwrap(), 2);
        assert_eq!(s.next_record_id(), 3); // monotonic, not num_records+1 by luck

        // deleteRecordStore + openRecordStore(create) resets ids.
        rms.delete_store("s").unwrap();
        assert!(rms.delete_store("s").is_err()); // gone now
        rms.open("s", true).unwrap();
        let s = rms.get_mut("s").unwrap();
        assert_eq!(s.next_record_id(), 1);
        assert_eq!(s.add_record(&[9], 0, 1).unwrap(), 1);
    }

    #[test]
    fn missing_record_and_bad_bounds_are_typed_errors_not_panics() {
        let mut rms = RmsRuntime::new();
        rms.open("s", true).unwrap();
        let s = rms.get_mut("s").unwrap();
        assert!(matches!(s.get_record(1), Err(JavaError::RecordStore(_))));
        assert!(matches!(
            s.get_record_size(1),
            Err(JavaError::RecordStore(_))
        ));
        // addRecord with a slice past the end is AIOOBE, never a slice panic.
        assert!(matches!(
            s.add_record(&[1, 2], 0, 5),
            Err(JavaError::ArrayIndexOutOfBounds { .. })
        ));
        assert!(matches!(
            s.add_record(&[1, 2], -1, 1),
            Err(JavaError::ArrayIndexOutOfBounds { .. })
        ));
    }

    #[test]
    fn close_and_delete_validate_presence() {
        let mut rms = RmsRuntime::new();
        assert!(rms.close("nope").is_err());
        rms.open("here", true).unwrap();
        assert!(rms.close("here").is_ok());
    }
}
