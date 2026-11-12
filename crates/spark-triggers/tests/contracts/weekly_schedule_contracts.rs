use serde_json::{json, Map, Value};
use spark_storage::{TriggerAction, TriggerDefinition, TriggerState};
use spark_triggers::state;

#[test]
fn weekly_trigger_skips_slots_before_creation_and_fires_at_first_slot_after() {
    // 2026-11-02 is a Monday; created before that day's 09:00 slot.
    let definition = weekly_definition("2026-11-02T01:58:00Z");
    let trigger_state = TriggerState::default();
    let at = |value: &str| state::parse_iso_datetime(Some(value)).expect("timestamp");

    assert_eq!(
        state::schedule_due_at(&definition, &trigger_state, at("2026-11-02T01:59:00Z")),
        None
    );
    assert_eq!(
        state::schedule_due_at(&definition, &trigger_state, at("2026-11-02T08:59:59Z")),
        None
    );
    assert_eq!(
        state::schedule_due_at(&definition, &trigger_state, at("2026-11-02T09:00:00Z")),
        Some(at("2026-11-02T09:00:00Z"))
    );
    assert_eq!(
        state::schedule_due_at(&definition, &trigger_state, at("2026-11-02T10:30:00Z")),
        Some(at("2026-11-02T09:00:00Z"))
    );
}

#[test]
fn weekly_trigger_created_after_this_weeks_slot_waits_for_next_week() {
    let definition = weekly_definition("2026-11-02T09:30:00Z");
    let trigger_state = TriggerState::default();
    let at = |value: &str| state::parse_iso_datetime(Some(value)).expect("timestamp");

    assert_eq!(
        state::schedule_due_at(&definition, &trigger_state, at("2026-11-05T12:00:00Z")),
        None
    );
    assert_eq!(
        state::schedule_due_at(&definition, &trigger_state, at("2026-11-09T09:00:00Z")),
        Some(at("2026-11-09T09:00:00Z"))
    );
}

#[test]
fn weekly_trigger_keeps_its_local_time_across_a_daylight_saving_change() {
    // Mondays 09:00 in New York: 13:00Z in summer time, 14:00Z after the 2026-11-01 change.
    let mut definition = weekly_definition("2026-10-20T00:00:00Z");
    definition
        .source
        .insert("timezone".to_string(), json!("America/New_York"));
    let mut trigger_state = TriggerState::default();
    let at = |value: &str| state::parse_iso_datetime(Some(value)).expect("timestamp");
    let next = |trigger_state: &TriggerState, now: &str| {
        state::compute_next_run_at_at(&definition, trigger_state, at(now))
    };

    assert_eq!(
        next(&trigger_state, "2026-10-25T00:00:00Z").as_deref(),
        Some("2026-10-26T13:00:00Z")
    );
    assert_eq!(
        state::schedule_due_at(&definition, &trigger_state, at("2026-10-26T12:59:59Z")),
        None
    );
    assert_eq!(
        state::schedule_due_at(&definition, &trigger_state, at("2026-10-26T13:00:00Z")),
        Some(at("2026-10-26T13:00:00Z"))
    );
    trigger_state.last_fired_at = Some("2026-10-26T13:00:00Z".to_string());
    assert_eq!(
        next(&trigger_state, "2026-10-26T13:00:01Z").as_deref(),
        Some("2026-11-02T14:00:00Z")
    );
    assert_eq!(
        state::schedule_due_at(&definition, &trigger_state, at("2026-11-02T13:59:59Z")),
        None
    );
    assert_eq!(
        state::schedule_due_at(&definition, &trigger_state, at("2026-11-02T14:00:00Z")),
        Some(at("2026-11-02T14:00:00Z"))
    );
}

#[test]
fn weekly_time_skipped_by_a_daylight_saving_change_runs_after_the_gap() {
    // 02:30 on 2026-03-08 does not exist in New York; it runs at 03:30 EDT.
    let mut definition = weekly_definition("2026-03-01T00:00:00Z");
    definition.source.extend([
        ("weekdays".to_string(), json!(["sun"])),
        ("hour".to_string(), json!(2)),
        ("minute".to_string(), json!(30)),
        ("timezone".to_string(), json!("America/New_York")),
    ]);
    let at = |value: &str| state::parse_iso_datetime(Some(value)).expect("timestamp");

    assert_eq!(
        state::compute_next_run_at_at(
            &definition,
            &TriggerState::default(),
            at("2026-03-02T00:00:00Z")
        )
        .as_deref(),
        Some("2026-03-08T07:30:00Z")
    );
}

#[test]
fn one_shot_run_at_honours_its_utc_offset() {
    let mut definition = weekly_definition("2026-11-02T00:00:00Z");
    definition.source = Map::from_iter([
        ("kind".to_string(), json!("once")),
        ("run_at".to_string(), json!("2030-01-01T10:00:00-05:00")),
    ]);
    let trigger_state = TriggerState::default();
    let at = |value: &str| state::parse_iso_datetime(Some(value)).expect("timestamp");

    assert_eq!(
        state::schedule_due_at(&definition, &trigger_state, at("2030-01-01T14:59:59Z")),
        None
    );
    assert_eq!(
        state::schedule_due_at(&definition, &trigger_state, at("2030-01-01T15:00:00Z")),
        Some(at("2030-01-01T15:00:00Z"))
    );
}

fn weekly_definition(created_at: &str) -> TriggerDefinition {
    TriggerDefinition {
        revision: String::new(),
        project_path: String::new(),
        id: "trigger-weekly".to_string(),
        name: "Weekly".to_string(),
        protected: false,
        source_type: "schedule".to_string(),
        action: TriggerAction {
            mission_id: None,
            role: None,
            mode: "static".to_string(),
            flow_name: "ops/run.dot".to_string(),
            static_context: Map::new(),
            execution_profile_id: None,
        },
        source: Map::from_iter([
            ("kind".to_string(), json!("weekly")),
            ("weekdays".to_string(), json!(["mon"])),
            ("hour".to_string(), json!(9)),
            ("minute".to_string(), json!(0)),
        ] as [(String, Value); 4]),
        created_at: created_at.to_string(),
        updated_at: created_at.to_string(),
    }
}
