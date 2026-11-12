use std::ops::RangeInclusive;

use jiff::tz::TimeZone;
use jiff::{civil, Span, Timestamp};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime, UtcOffset};

use crate::models::{
    TriggerDefinition, TriggerState, TriggerStateHistoryEntry, SOURCE_POLL, SOURCE_SCHEDULE,
};

const WEEKDAYS: &[&str] = &["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
pub const MAX_RECENT_HISTORY_ENTRIES: usize = 20;

pub fn refresh_next_run_at(
    definition: &TriggerDefinition,
    state: &mut TriggerState,
) -> Option<String> {
    let next_run_at = compute_next_run_at(definition, state);
    state.next_run_at = next_run_at.clone();
    next_run_at
}

pub fn record_success(
    state: &mut TriggerState,
    timestamp: impl Into<String>,
    message: impl Into<String>,
    run_id: Option<String>,
) {
    let timestamp = timestamp.into();
    let message = message.into();
    state.last_error = None;
    state.last_fired_at = Some(timestamp.clone());
    state.last_result = Some("success".to_string());
    push_history(
        state,
        TriggerStateHistoryEntry {
            timestamp,
            status: "success".to_string(),
            message,
            run_id,
        },
    );
}

pub fn record_failure(
    state: &mut TriggerState,
    timestamp: impl Into<String>,
    message: impl Into<String>,
) {
    let timestamp = timestamp.into();
    let message = message.into();
    state.last_error = Some(message.clone());
    state.last_fired_at = Some(timestamp.clone());
    state.last_result = Some("failed".to_string());
    push_history(
        state,
        TriggerStateHistoryEntry {
            timestamp,
            status: "failed".to_string(),
            message,
            run_id: None,
        },
    );
}

pub fn push_history(state: &mut TriggerState, entry: TriggerStateHistoryEntry) {
    state.recent_history.insert(0, entry);
    if state.recent_history.len() > MAX_RECENT_HISTORY_ENTRIES {
        state.recent_history.truncate(MAX_RECENT_HISTORY_ENTRIES);
    }
}

pub fn compute_next_run_at(definition: &TriggerDefinition, state: &TriggerState) -> Option<String> {
    compute_next_run_at_at(definition, state, OffsetDateTime::now_utc())
}

pub fn compute_next_run_at_at(
    definition: &TriggerDefinition,
    state: &TriggerState,
    now: OffsetDateTime,
) -> Option<String> {
    match definition.source_type.as_str() {
        SOURCE_SCHEDULE => compute_schedule_next_run_at(definition, state, now),
        SOURCE_POLL => state.next_run_at.clone().or_else(|| {
            let interval = definition
                .source
                .get("interval_seconds")
                .and_then(|value| value.as_i64())?;
            Some(datetime_to_iso(now + Duration::seconds(interval)))
        }),
        _ => None,
    }
}

pub fn schedule_due_at(
    definition: &TriggerDefinition,
    state: &TriggerState,
    now: OffsetDateTime,
) -> Option<OffsetDateTime> {
    if definition.source_type != SOURCE_SCHEDULE {
        return None;
    }
    let kind = definition
        .source
        .get("kind")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let last_fired_at = parse_iso_datetime(state.last_fired_at.as_deref());
    match kind {
        "once" => {
            if last_fired_at.is_some() {
                return None;
            }
            let run_at = definition
                .source
                .get("run_at")
                .and_then(|value| value.as_str())
                .and_then(|value| parse_iso_datetime(Some(value)))?;
            (now >= run_at).then_some(run_at)
        }
        "interval" => {
            let interval = definition
                .source
                .get("interval_seconds")
                .and_then(|value| value.as_i64())?;
            // A new trigger first falls due one interval after it was created.
            let Some(base) =
                last_fired_at.or_else(|| parse_iso_datetime(Some(&definition.created_at)))
            else {
                return Some(now);
            };
            let next_due = base + Duration::seconds(interval);
            (now >= next_due).then_some(next_due)
        }
        "weekly" => {
            let scheduled = weekly_most_recent_scheduled_time(definition, now)?;
            let not_yet_fired = match last_fired_at {
                Some(last) => last < scheduled,
                // Never fire for a slot that passed before the trigger existed.
                None => parse_iso_datetime(Some(&definition.created_at))
                    .is_none_or(|created| created <= scheduled),
            };
            (scheduled <= now && not_yet_fired).then_some(scheduled)
        }
        _ => None,
    }
}

pub fn record_activation_success(
    definition: &TriggerDefinition,
    state: &mut TriggerState,
    timestamp: OffsetDateTime,
    message: impl Into<String>,
    run_id: Option<String>,
) {
    record_success(state, datetime_to_iso(timestamp), message, run_id);
    state.next_run_at = compute_next_run_at_at(definition, state, timestamp);
}

pub fn record_activation_failure(
    definition: &TriggerDefinition,
    state: &mut TriggerState,
    timestamp: OffsetDateTime,
    message: impl Into<String>,
) {
    record_failure(state, datetime_to_iso(timestamp), message);
    state.next_run_at = compute_next_run_at_at(definition, state, timestamp);
}

pub fn record_activation_no_op(
    definition: &TriggerDefinition,
    state: &mut TriggerState,
    timestamp: OffsetDateTime,
    message: impl Into<String>,
) {
    let next_run_base = timestamp;
    let timestamp = datetime_to_iso(timestamp);
    let message = message.into();
    state.last_error = Some(message.clone());
    state.last_fired_at = Some(timestamp.clone());
    state.last_result = Some("success".to_string());
    push_history(
        state,
        TriggerStateHistoryEntry {
            timestamp,
            status: "success".to_string(),
            message,
            run_id: None,
        },
    );
    state.next_run_at = compute_next_run_at_at(definition, state, next_run_base);
}

fn compute_schedule_next_run_at(
    definition: &TriggerDefinition,
    state: &TriggerState,
    now: OffsetDateTime,
) -> Option<String> {
    let kind = definition
        .source
        .get("kind")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    match kind {
        "once" => {
            if state
                .last_fired_at
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .is_some()
            {
                None
            } else {
                definition
                    .source
                    .get("run_at")
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_string)
            }
        }
        "interval" => {
            let interval = definition
                .source
                .get("interval_seconds")
                .and_then(|value| value.as_i64())?;
            let base = parse_iso_datetime(state.last_fired_at.as_deref())
                .or_else(|| parse_iso_datetime(Some(&definition.created_at)))
                .unwrap_or(now);
            Some(datetime_to_iso(base + Duration::seconds(interval)))
        }
        "weekly" => weekly_next_run_at(definition, now),
        _ => None,
    }
}

fn weekly_next_run_at(definition: &TriggerDefinition, now: OffsetDateTime) -> Option<String> {
    weekly_scheduled_time(definition, now).map(datetime_to_iso)
}

fn weekly_scheduled_time(
    definition: &TriggerDefinition,
    now: OffsetDateTime,
) -> Option<OffsetDateTime> {
    let now = now.replace_nanosecond(0).ok()?;
    weekly_slots(definition, now, 0..=7)?
        .into_iter()
        .find(|slot| *slot >= now)
}

fn weekly_most_recent_scheduled_time(
    definition: &TriggerDefinition,
    now: OffsetDateTime,
) -> Option<OffsetDateTime> {
    weekly_slots(definition, now, -7..=0)?
        .into_iter()
        .rev()
        .find(|slot| *slot <= now)
}

/// The trigger's slots on the given days from `now`'s date, earliest first. Days and time
/// are in the source's IANA `timezone`, else UTC. A time a daylight-saving change skips
/// runs after the gap; one it repeats runs at the first of the two.
fn weekly_slots(
    definition: &TriggerDefinition,
    now: OffsetDateTime,
    days: RangeInclusive<i64>,
) -> Option<Vec<OffsetDateTime>> {
    let source = &definition.source;
    let weekdays = source
        .get("weekdays")?
        .as_array()?
        .iter()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>();
    let time = civil::Time::new(
        i8::try_from(source.get("hour")?.as_i64()?).ok()?,
        i8::try_from(source.get("minute")?.as_i64()?).ok()?,
        0,
        0,
    )
    .ok()?;
    let zone = match source.get("timezone").and_then(|value| value.as_str()) {
        Some(name) => TimeZone::get(name).ok()?,
        None => TimeZone::UTC,
    };
    let today = Timestamp::from_second(now.unix_timestamp())
        .ok()?
        .to_zoned(zone.clone())
        .date();
    let mut slots = Vec::new();
    for offset in days {
        let day = today.checked_add(Span::new().days(offset)).ok()?;
        if !weekdays.contains(&WEEKDAYS[day.weekday().to_monday_zero_offset() as usize]) {
            continue;
        }
        let slot = day.to_datetime(time).to_zoned(zone.clone()).ok()?;
        slots.push(OffsetDateTime::from_unix_timestamp(slot.timestamp().as_second()).ok()?);
    }
    Some(slots)
}

pub fn parse_iso_datetime(value: Option<&str>) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value?.trim(), &Rfc3339)
        .ok()
        .map(|value| value.to_offset(UtcOffset::UTC))
}

pub fn datetime_to_iso(value: OffsetDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        value.year(),
        u8::from(value.month()),
        value.day(),
        value.hour(),
        value.minute(),
        value.second()
    )
}
