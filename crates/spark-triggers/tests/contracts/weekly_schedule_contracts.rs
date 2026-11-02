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
