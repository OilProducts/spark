//! Incremental combined-journal cache.
//!
//! Live consumers (the publisher, cursor replay, gate/question projections)
//! only ever need entries appended since a recent combined-journal sequence,
//! but rebuilding the combined journal parses every source event log from
//! zero. This cache keeps, per active parent run: a byte offset into each
//! source `events.jsonl`, a bounded ring of the most recent combined
//! entries, and the incrementally-applied segment projection. A refresh
//! parses only appended bytes; deep-history requests fall back to the cold
//! rebuild (`combined_run_journal_entries`), which shares its entry
//! construction, ordering, and stamping with this module.
//!
//! Memory is bounded by design: the ring is capped, the segment projection
//! is bounded by transcript size, at most `CACHE_CAPACITY` runs are cached,
//! and terminal runs are evicted. The cache never retains a full parsed
//! journal.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use attractor_core::JournalEntry;
use serde_json::Value;

use crate::error::Result;
use crate::events::read_raw_events_after;
use crate::journals::{
    decorate_child_journal_entry, journal_entry_from_event, journal_replay_order, replay_key,
    stamp_combined_sequence,
};
use crate::paths::RunRootPaths;
use crate::segments::SegmentProjectionState;
use crate::store::RunStore;

const RING_CAPACITY: usize = 4096;
const CACHE_CAPACITY: usize = 8;

/// A cursor-relative view of a run's combined journal.
#[derive(Debug)]
pub struct CombinedJournalWindow {
    /// Highest combined sequence currently known (0 when the journal is empty).
    pub latest_sequence: u64,
    /// Entries with combined sequence strictly greater than the cursor;
    /// meaningful only when `complete` is true.
    pub entries_after: Vec<JournalEntry>,
    /// False when the cursor predates the retained ring: the caller must
    /// serve this request from the cold rebuild instead.
    pub complete: bool,
    /// Projected segments touched after the cursor.
    pub segments_after: Vec<Value>,
}

struct SourceCursor {
    paths: RunRootPaths,
    offset: u64,
    /// `None` for the parent; child entries are decorated with lineage.
    child_record: Option<attractor_core::RunRecord>,
}

struct CachedCombined {
    parent_run_id: String,
    sources: Vec<SourceCursor>,
    ring: VecDeque<JournalEntry>,
    next_sequence: u64,
    last_key: Option<(String, u8, u64, String)>,
    segments: SegmentProjectionState,
    /// Set when a parent event announces a child launch; cleared once
    /// discovery finds a new source.
    pending_discovery: bool,
    touched: u64,
}

#[derive(Default)]
struct CombinedJournalCache {
    runs: HashMap<PathBuf, CachedCombined>,
    clock: u64,
}

fn cache() -> &'static Mutex<CombinedJournalCache> {
    static CACHE: OnceLock<Mutex<CombinedJournalCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(CombinedJournalCache::default()))
}

/// Serves a cursor window over the combined journal, parsing only bytes
/// appended since the last call for this run. Returns `Ok(None)` for an
/// unknown run. A window with `complete: false` means the cursor is older
/// than the retained ring — the caller should fall back to
/// `combined_run_journal_entries`.
pub fn combined_journal_window(
    store: &RunStore,
    run_id: &str,
    after: u64,
) -> Result<Option<CombinedJournalWindow>> {
    let Some(parent_paths) = store.find_run_root(run_id)? else {
        // Unknown run: drop any cache entry left behind by a deleted root.
        evict_combined_journal(run_id);
        return Ok(None);
    };
    let key = parent_paths.root.clone();
    let mut guard = cache().lock().expect("combined journal cache poisoned");
    let cache = &mut *guard;
    cache.clock += 1;
    let clock = cache.clock;

    if !cache.runs.contains_key(&key) {
        let entry = CachedCombined {
            parent_run_id: run_id.to_string(),
            sources: vec![SourceCursor {
                paths: parent_paths.clone(),
                offset: 0,
                child_record: None,
            }],
            ring: VecDeque::new(),
            next_sequence: 1,
            last_key: None,
            segments: SegmentProjectionState::default(),
            pending_discovery: true,
            touched: clock,
        };
        cache.runs.insert(key.clone(), entry);
        while cache.runs.len() > CACHE_CAPACITY {
            let Some(oldest) = cache
                .runs
                .iter()
                .min_by_key(|(_, cached)| cached.touched)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            cache.runs.remove(&oldest);
        }
    }
    let cached = cache.runs.get_mut(&key).expect("entry just ensured");
    cached.touched = clock;

    if !refresh(store, cached)? {
        // Out-of-order or shrunken source: renumbering would rewrite
        // history, so rebuild from scratch. The fresh numbering makes any
        // stale caller cursor non-contiguous, which callers already treat
        // as a resync.
        cache.runs.remove(&key);
        drop(guard);
        return combined_journal_window_cold(store, run_id, after);
    }

    let latest_sequence = cached.next_sequence.saturating_sub(1);
    let ring_start = cached.ring.front().map(|entry| entry.sequence);
    let complete = match ring_start {
        None => true,
        Some(start) => after.saturating_add(1) >= start || after >= latest_sequence,
    };
    let entries_after = if complete {
        cached
            .ring
            .iter()
            .filter(|entry| entry.sequence > after)
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    let segments_after = cached.segments.segments_touched_after(after);
    Ok(Some(CombinedJournalWindow {
        latest_sequence,
        entries_after,
        complete,
        segments_after,
    }))
}

/// Drops a run's cache entry (call when the run reaches a terminal status).
pub fn evict_combined_journal(run_id: &str) {
    let mut guard = cache().lock().expect("combined journal cache poisoned");
    guard
        .runs
        .retain(|_, cached| cached.parent_run_id != run_id);
}

fn combined_journal_window_cold(
    store: &RunStore,
    run_id: &str,
    after: u64,
) -> Result<Option<CombinedJournalWindow>> {
    let Some(entries) = crate::journals::combined_run_journal_entries(store, run_id)? else {
        return Ok(None);
    };
    let latest_sequence = entries
        .iter()
        .map(|entry| entry.sequence)
        .max()
        .unwrap_or(0);
    let projection = crate::segments::project_run_segments(&entries);
    let segments_after = projection
        .segments
        .into_iter()
        .filter(|segment| {
            segment
                .get("latest_sequence")
                .and_then(Value::as_u64)
                .is_some_and(|sequence| sequence > after)
        })
        .collect();
    let entries_after = entries
        .into_iter()
        .filter(|entry| entry.sequence > after)
        .collect();
    Ok(Some(CombinedJournalWindow {
        latest_sequence,
        entries_after,
        complete: true,
        segments_after,
    }))
}

/// Ingests appended events from every source. Returns false when the cache
/// can no longer extend history consistently (shrunken file or an append
/// that sorts before already-stamped entries) and must be rebuilt.
fn refresh(store: &RunStore, cached: &mut CachedCombined) -> Result<bool> {
    if cached.pending_discovery && discover_children(store, cached)? {
        cached.pending_discovery = false;
    }

    let mut fresh: Vec<JournalEntry> = Vec::new();
    for source in &mut cached.sources {
        let Some((events, consumed)) = read_raw_events_after(&source.paths, source.offset)? else {
            return Ok(false);
        };
        source.offset = consumed;
        for event in &events {
            let Some(mut entry) = journal_entry_from_event(event) else {
                continue;
            };
            match &source.child_record {
                None => {
                    if event.event_type == "ChildRunStarted" {
                        cached.pending_discovery = true;
                    }
                }
                Some(record) => decorate_child_journal_entry(&mut entry, Some(record)),
            }
            fresh.push(entry);
        }
    }
    if fresh.is_empty() {
        return Ok(true);
    }
    fresh.sort_by(journal_replay_order);
    for mut entry in fresh {
        let key = replay_key(&entry);
        if cached
            .last_key
            .as_ref()
            .is_some_and(|last_key| key < *last_key)
        {
            return Ok(false);
        }
        cached.last_key = Some(key);
        stamp_combined_sequence(&mut entry, cached.next_sequence);
        cached.next_sequence = cached.next_sequence.saturating_add(1);
        cached.segments.apply(&entry);
        cached.ring.push_back(entry);
        while cached.ring.len() > RING_CAPACITY {
            cached.ring.pop_front();
        }
    }
    Ok(true)
}

/// Adds sources for children not yet tracked, in the same order the cold
/// rebuild visits them. Returns true when a new child was found.
fn discover_children(store: &RunStore, cached: &mut CachedCombined) -> Result<bool> {
    let mut children: Vec<(RunRootPaths, attractor_core::RunRecord)> = Vec::new();
    for paths in store.list_existing_run_roots()? {
        if cached
            .sources
            .iter()
            .any(|source| source.paths.root == paths.root)
        {
            continue;
        }
        let Some(record) = store.read_run_record(&paths)? else {
            continue;
        };
        if record.parent_run_id.as_deref() != Some(cached.parent_run_id.as_str()) {
            continue;
        }
        children.push((paths, record));
    }
    if children.is_empty() {
        return Ok(false);
    }
    children.sort_by(|left, right| {
        let left_index = left.1.child_invocation_index.unwrap_or(0);
        let right_index = right.1.child_invocation_index.unwrap_or(0);
        left_index
            .cmp(&right_index)
            .then_with(|| left.1.run_id.cmp(&right.1.run_id))
    });
    for (paths, record) in children {
        cached.sources.push(SourceCursor {
            paths,
            offset: 0,
            child_record: Some(record),
        });
    }
    Ok(true)
}
