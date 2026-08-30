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
//! `getRecordSize(id)`, `getRecord(id)`, `addRecord`, `setRecord`, and
//! `deleteRecord`. Record IDs are
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
#[derive(Debug, Clone, PartialEq, Eq)]
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

    /// `setRecord(id, data, offset, numBytes)` replaces a record without
    /// changing its id or the next-id sequence.
    pub fn set_record(
        &mut self,
        id: i32,
        data: &[i8],
        offset: i32,
        num_bytes: i32,
    ) -> Result<(), JavaError> {
        if !self.records.contains_key(&id) {
            return Err(invalid_id(id));
        }
        self.records
            .insert(id, slice_checked(data, offset, num_bytes)?.to_vec());
        Ok(())
    }

    /// `deleteRecord(id)`. Deleted identifiers are never reused.
    pub fn delete_record(&mut self, id: i32) -> Result<(), JavaError> {
        self.records
            .remove(&id)
            .map(|_| ())
            .ok_or_else(|| invalid_id(id))
    }

    pub fn size_bytes(&self) -> u64 {
        self.records
            .values()
            .map(|record| record.len() as u64)
            .sum()
    }

    fn snapshot(&self) -> RecordStoreSnapshot {
        RecordStoreSnapshot {
            next_id: self.next_id,
            records: self.records.clone(),
        }
    }

    fn from_snapshot(snapshot: RecordStoreSnapshot) -> Result<Self, JavaError> {
        let maximum_id = snapshot.records.keys().next_back().copied().unwrap_or(0);
        if snapshot.next_id < 1
            || snapshot.records.keys().any(|id| *id < 1)
            || snapshot.next_id <= maximum_id
        {
            return Err(JavaError::RecordStore(
                "invalid persisted record-id sequence".to_owned(),
            ));
        }
        Ok(Self {
            records: snapshot.records,
            next_id: snapshot.next_id,
        })
    }
}

/// Stable host-facing representation of one store. This is not a wire format;
/// host crates may encode it however they choose while preserving MIDP ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordStoreSnapshot {
    pub next_id: i32,
    pub records: BTreeMap<i32, Vec<i8>>,
}

/// Deterministically ordered snapshot of the whole RMS namespace.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RmsSnapshot {
    pub stores: BTreeMap<String, RecordStoreSnapshot>,
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
    open_counts: HashMap<String, u32>,
    capacity_bytes: Option<u64>,
}

impl RmsRuntime {
    /// A fresh, empty RMS namespace.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a namespace with the reviewed device profile's RMS quota.
    pub fn with_capacity(capacity_bytes: Option<u64>) -> Self {
        Self {
            capacity_bytes,
            ..Self::default()
        }
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
        *self.open_counts.entry(name.to_owned()).or_default() += 1;
        Ok(name.to_string())
    }

    /// `closeRecordStore()` — closes the handle. The in-memory store persists in
    /// the namespace (a real device keeps the record store across a close); this
    /// only validates the store exists, matching MIDP's throw-if-absent.
    pub fn close(&mut self, name: &str) -> Result<(), JavaError> {
        let count = self.open_counts.get_mut(name).ok_or_else(|| {
            JavaError::RecordStore(format!("RecordStoreNotOpenException: {name}"))
        })?;
        if *count == 0 {
            return Err(JavaError::RecordStore(format!(
                "RecordStoreNotOpenException: {name}"
            )));
        }
        *count -= 1;
        Ok(())
    }

    /// `RecordStore.deleteRecordStore(name)` — removes the store (and its ids)
    /// entirely; throws `RecordStoreNotFoundException` if it does not exist.
    pub fn delete_store(&mut self, name: &str) -> Result<(), JavaError> {
        let result = self
            .stores
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| not_found(name));
        self.open_counts.remove(name);
        result
    }

    /// Whether a record store with this name exists (backs an `exists()` probe).
    pub fn contains(&self, name: &str) -> bool {
        self.stores.contains_key(name)
    }

    /// `RecordStore.listRecordStores()`, deterministically sorted. An empty
    /// vector corresponds to the Java API's `null` result.
    pub fn list_record_stores(&self) -> Vec<String> {
        let mut names: Vec<_> = self.stores.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn used_bytes(&self) -> u64 {
        self.stores.values().map(RecordStore::size_bytes).sum()
    }

    /// `getSizeAvailable`, saturated to Java's signed integer range.
    pub fn size_available(&self) -> i32 {
        let remaining = self
            .capacity_bytes
            .map(|capacity| capacity.saturating_sub(self.used_bytes()))
            .unwrap_or(i32::MAX as u64);
        remaining.min(i32::MAX as u64) as i32
    }

    fn require_space(&self, growth: u64) -> Result<(), JavaError> {
        if self
            .capacity_bytes
            .is_some_and(|capacity| self.used_bytes().saturating_add(growth) > capacity)
        {
            Err(JavaError::RecordStore(
                "RecordStoreFullException: RMS capacity exceeded".to_owned(),
            ))
        } else {
            Ok(())
        }
    }

    /// Quota-aware host dispatch for `addRecord`.
    pub fn add_record(
        &mut self,
        name: &str,
        data: &[i8],
        offset: i32,
        num_bytes: i32,
    ) -> Result<i32, JavaError> {
        let record = slice_checked(data, offset, num_bytes)?;
        self.require_space(record.len() as u64)?;
        self.get_mut(name)?.add_record(data, offset, num_bytes)
    }

    /// Quota-aware host dispatch for `setRecord`.
    pub fn set_record(
        &mut self,
        name: &str,
        id: i32,
        data: &[i8],
        offset: i32,
        num_bytes: i32,
    ) -> Result<(), JavaError> {
        let new_len = slice_checked(data, offset, num_bytes)?.len() as u64;
        let old_len = self.get(name)?.get_record_size(id)? as u64;
        self.require_space(new_len.saturating_sub(old_len))?;
        self.get_mut(name)?.set_record(id, data, offset, num_bytes)
    }

    pub fn delete_record(&mut self, name: &str, id: i32) -> Result<(), JavaError> {
        self.get_mut(name)?.delete_record(id)
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

    /// Export every store without imposing a host filesystem format.
    pub fn snapshot(&self) -> RmsSnapshot {
        RmsSnapshot {
            stores: self
                .stores
                .iter()
                .map(|(name, store)| (name.clone(), store.snapshot()))
                .collect(),
        }
    }

    /// Restore a host snapshot after validating every monotonic record-id
    /// sequence. Invalid host data becomes a typed `RecordStoreException`.
    pub fn from_snapshot(snapshot: RmsSnapshot) -> Result<Self, JavaError> {
        Self::from_snapshot_with_capacity(snapshot, None)
    }

    pub fn from_snapshot_with_capacity(
        snapshot: RmsSnapshot,
        capacity_bytes: Option<u64>,
    ) -> Result<Self, JavaError> {
        let stores = snapshot
            .stores
            .into_iter()
            .map(|(name, store)| Ok((name, RecordStore::from_snapshot(store)?)))
            .collect::<Result<HashMap<_, _>, JavaError>>()?;
        let runtime = Self {
            stores,
            open_counts: HashMap::new(),
            capacity_bytes,
        };
        runtime.require_space(0)?;
        Ok(runtime)
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

    #[test]
    fn host_snapshot_round_trips_monotonic_record_ids() {
        let mut rms = RmsRuntime::new();
        rms.open("slot", true).unwrap();
        rms.get_mut("slot")
            .unwrap()
            .add_record(&[-1, 2], 0, 2)
            .unwrap();
        rms.get_mut("slot").unwrap().add_record(&[3], 0, 1).unwrap();

        let snapshot = rms.snapshot();
        let restored = RmsRuntime::from_snapshot(snapshot.clone()).unwrap();
        assert_eq!(restored.snapshot(), snapshot);
        assert_eq!(restored.get("slot").unwrap().next_record_id(), 3);

        let mut invalid = snapshot;
        invalid.stores.get_mut("slot").unwrap().next_id = 2;
        assert!(RmsRuntime::from_snapshot(invalid).is_err());
    }

    #[test]
    fn set_delete_list_and_quota_follow_midp_semantics() {
        let mut rms = RmsRuntime::with_capacity(Some(5));
        rms.open("z", true).unwrap();
        rms.open("a", true).unwrap();
        assert_eq!(rms.list_record_stores(), vec!["a", "z"]);
        let id = rms.add_record("z", &[1, 2, 3], 0, 3).unwrap();
        assert_eq!(rms.size_available(), 2);
        assert!(rms.add_record("z", &[4, 5, 6], 0, 3).is_err());
        rms.set_record("z", id, &[8, 9], 0, 2).unwrap();
        assert_eq!(rms.get("z").unwrap().get_record(id).unwrap(), vec![8, 9]);
        assert_eq!(rms.size_available(), 3);
        rms.delete_record("z", id).unwrap();
        assert_eq!(rms.get("z").unwrap().next_record_id(), 2);
        assert_eq!(rms.size_available(), 5);
    }
}
