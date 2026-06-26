use time::{Duration, OffsetDateTime, Weekday};

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
            let Some(last_fired_at) = last_fired_at else {
                return Some(now);
            };
            let next_due = last_fired_at + Duration::seconds(interval);
            (now >= next_due).then_some(next_due)
        }
        "weekly" => {
            let scheduled = weekly_most_recent_scheduled_time(definition, now)?;
            (scheduled <= now && last_fired_at.is_none_or(|last| last < scheduled))
                .then_some(scheduled)
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
            let base = parse_iso_datetime(state.last_fired_at.as_deref()).unwrap_or(now);
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
    let weekdays = definition
        .source
        .get("weekdays")
        .and_then(|value| value.as_array())?
        .iter()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>();
    let hour = definition.source.get("hour")?.as_i64()? as u8;
    let minute = definition.source.get("minute")?.as_i64()? as u8;
    for offset in 0..8 {
        let candidate = now + Duration::days(offset);
        let weekday = weekday_name(candidate.weekday());
        if !weekdays.contains(&weekday) {
            continue;
        }
        let scheduled = candidate.replace_time(time::Time::from_hms(hour, minute, 0).ok()?);
        let scheduled = scheduled.replace_nanosecond(0).ok()?;
        if offset > 0 || scheduled >= now.replace_nanosecond(0).ok()? {
            return Some(scheduled);
        }
    }
    None
}

fn weekly_most_recent_scheduled_time(
    definition: &TriggerDefinition,
    now: OffsetDateTime,
) -> Option<OffsetDateTime> {
    let weekdays = definition
        .source
        .get("weekdays")
        .and_then(|value| value.as_array())?
        .iter()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>();
    let hour = definition.source.get("hour")?.as_i64()? as u8;
    let minute = definition.source.get("minute")?.as_i64()? as u8;
    for offset in 0..8 {
        let candidate = now - Duration::days(offset);
        let weekday = weekday_name(candidate.weekday());
        if !weekdays.contains(&weekday) {
            continue;
        }
        let scheduled = candidate.replace_time(time::Time::from_hms(hour, minute, 0).ok()?);
        let scheduled = scheduled.replace_nanosecond(0).ok()?;
        if scheduled <= now {
            return Some(scheduled);
        }
    }
    None
}

fn weekday_name(value: Weekday) -> &'static str {
    match value {
        Weekday::Monday => WEEKDAYS[0],
        Weekday::Tuesday => WEEKDAYS[1],
        Weekday::Wednesday => WEEKDAYS[2],
        Weekday::Thursday => WEEKDAYS[3],
        Weekday::Friday => WEEKDAYS[4],
        Weekday::Saturday => WEEKDAYS[5],
        Weekday::Sunday => WEEKDAYS[6],
    }
}

pub fn parse_iso_datetime(value: Option<&str>) -> Option<OffsetDateTime> {
    let value = value?.trim();
    if value.len() < 19 {
        return None;
    }
    let year = value.get(0..4)?.parse::<i32>().ok()?;
    let month = value.get(5..7)?.parse::<u8>().ok()?;
    let day = value.get(8..10)?.parse::<u8>().ok()?;
    let hour = value.get(11..13)?.parse::<u8>().ok()?;
    let minute = value.get(14..16)?.parse::<u8>().ok()?;
    let second = value.get(17..19)?.parse::<u8>().ok()?;
    let date = time::Date::from_calendar_date(year, month.try_into().ok()?, day).ok()?;
    let time = time::Time::from_hms(hour, minute, second).ok()?;
    Some(OffsetDateTime::new_utc(date, time))
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
