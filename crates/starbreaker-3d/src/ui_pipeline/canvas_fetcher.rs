//! DataCore canvas fetcher with an O(1) name→GUID index (B2b) and a per-binding
//! memoising bytes cache (B2a).
//!
//! [`DatacoreCanvasFetcher`] precomputes a `HashMap<lowercase_key, CigGuid>` over
//! all UI-support record families once at construction time, turning every
//! `fetch_canvas_by_name` call from O(records) to O(1), and memoises each
//! record's compact-JSON bytes so the DataCore traversal runs once per record.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use starbreaker_datacore::Database;
use starbreaker_datacore::starbreaker_common::CigGuid;
use starbreaker_ui::{UiError, pipeline::CanvasFetcher, record_name::extract_record_name};

use super::{datacore_ui_lookup_type_names, parse_guid};

// ── Name index ────────────────────────────────────────────────────────────────

/// Precomputed, case-insensitive name→GUID index over all DataCore UI record families.
struct CanvasNameIndex {
    index: HashMap<String, CigGuid>,
}

impl CanvasNameIndex {
    fn build(db: &Database<'_>) -> Self {
        let mut index: HashMap<String, CigGuid> = HashMap::new();
        // Track keys already warned about to emit at most one warning per duplicate name,
        // even when the same name appears in many records of the same type.
        let mut warned: std::collections::HashSet<String> = std::collections::HashSet::new();
        for type_name in datacore_ui_lookup_type_names() {
            for record in db.records_by_type_name(type_name) {
                let full_name = db.resolve_string2(record.name_offset);
                let stem = full_name.rsplit('.').next().unwrap_or(full_name);
                let stem_lower = stem.to_ascii_lowercase();
                let full_lower = full_name.to_ascii_lowercase();
                // Iterate types in declared priority order; first type+record wins.
                for key in std::iter::once(&stem_lower).chain(
                    (full_lower != stem_lower).then_some(&full_lower),
                ) {
                    match index.entry(key.clone()) {
                        std::collections::hash_map::Entry::Vacant(e) => {
                            e.insert(record.id);
                        }
                        std::collections::hash_map::Entry::Occupied(e) if *e.get() != record.id => {
                            // Duplicate stems are inherent to the shipped
                            // DataCore (e.g. 'ar_boxout_base' exists in
                            // several UI record families); first-wins is the
                            // established policy, so the per-name detail is
                            // debug-level diagnostics, not a warning.
                            if warned.insert(key.clone()) {
                                log::debug!(
                                    "ui_pipeline: duplicate UI record name '{}' in {}; first wins",
                                    key, type_name
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        if !warned.is_empty() {
            log::debug!(
                "ui_pipeline: {} duplicate UI record name(s) resolved first-wins (enable debug logging for the list)",
                warned.len()
            );
        }
        Self { index }
    }

    fn lookup(&self, name: &str) -> Option<CigGuid> {
        self.index.get(&name.to_ascii_lowercase()).copied()
    }
}

// ── Fetcher ───────────────────────────────────────────────────────────────────

/// Canvas fetcher backed by a DataCore [`Database`] with an O(1) name index and a
/// per-binding memoising cache of parsed JSON values (B2a).
///
/// The cache stores each record's parsed [`serde_json::Value`] behind an [`Rc`]
/// keyed by GUID, so the expensive DataCore traversal (`to_json_compact`) *and*
/// the JSON parse each run at most once per record per binding render. The owned
/// `fetch_canvas_*` methods deep-clone on hit (the trait's owned-`Value` contract);
/// [`CanvasFetcher::fetch_canvas_by_path_shared`] returns the `Rc` directly, so
/// tag resolution — which fetches the large `TagDatabase` thousands of times per
/// heavy binding — pays only a refcount bump instead of a deep clone.
pub(super) struct DatacoreCanvasFetcher<'a> {
    db: &'a Database<'a>,
    name_index: CanvasNameIndex,
    /// GUID → parsed JSON value, memoised for the lifetime of one fetcher (one
    /// binding render). The fetcher is never shared across threads.
    by_guid: RefCell<HashMap<CigGuid, Rc<serde_json::Value>>>,
}

impl<'a> DatacoreCanvasFetcher<'a> {
    pub(super) fn new(db: &'a Database<'a>) -> Self {
        Self {
            db,
            name_index: CanvasNameIndex::build(db),
            by_guid: RefCell::new(HashMap::new()),
        }
    }

    fn fetch_by_guid_rc(&self, cig_guid: CigGuid, lookup_key: &str) -> Result<Rc<serde_json::Value>, UiError> {
        if let Some(value) = self.by_guid.borrow().get(&cig_guid) {
            return Ok(Rc::clone(value));
        }
        let record = self.db.record_by_id(&cig_guid).ok_or_else(|| UiError::FetchFailed {
            guid: lookup_key.to_string(),
            source: format!("record not found in DataCore for GUID {lookup_key}").into(),
        })?;
        let bytes = starbreaker_datacore::export::to_json_compact(self.db, record).map_err(|e| {
            UiError::FetchFailed {
                guid: lookup_key.to_string(),
                source: Box::new(e),
            }
        })?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| UiError::FetchFailed {
            guid: lookup_key.to_string(),
            source: Box::new(e),
        })?;
        let rc = Rc::new(value);
        self.by_guid.borrow_mut().insert(cig_guid, Rc::clone(&rc));
        Ok(rc)
    }

    fn fetch_by_guid(&self, cig_guid: CigGuid, lookup_key: &str) -> Result<serde_json::Value, UiError> {
        self.fetch_by_guid_rc(cig_guid, lookup_key).map(|rc| (*rc).clone())
    }
}

impl<'a> CanvasFetcher for DatacoreCanvasFetcher<'a> {
    fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, UiError> {
        let cig_guid = parse_guid(guid).ok_or_else(|| UiError::FetchFailed {
            guid: guid.to_string(),
            source: "invalid GUID format".into(),
        })?;
        self.fetch_by_guid(cig_guid, guid)
    }

    fn fetch_canvas_by_name(&self, record_name: &str) -> Result<serde_json::Value, UiError> {
        let cig_guid = self.name_index.lookup(record_name).ok_or_else(|| UiError::FetchFailed {
            guid: record_name.to_string(),
            source: format!("no UI-support record found by name: {record_name}").into(),
        })?;
        self.fetch_by_guid(cig_guid, record_name)
    }

    fn fetch_canvas_by_path_shared(&self, path_or_name: &str) -> Result<Rc<serde_json::Value>, UiError> {
        // Mirror the trait default `fetch_canvas_by_path` resolution
        // (extract_record_name → name index) but return the cached `Rc` directly,
        // so repeated `TagDatabase` fetches in tag resolution are a refcount bump.
        let name = extract_record_name(path_or_name);
        let cig_guid = self.name_index.lookup(&name).ok_or_else(|| UiError::FetchFailed {
            guid: path_or_name.to_string(),
            source: format!("no UI-support record found by name: {name}").into(),
        })?;
        self.fetch_by_guid_rc(cig_guid, path_or_name)
    }
}
