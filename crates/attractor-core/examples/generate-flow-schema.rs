#![forbid(unsafe_code)]

fn main() {
    let schema = attractor_core::flow_definition_schema_value();
    println!(
        "{}",
        serde_json::to_string_pretty(&schema).expect("schema serializes")
    );
}
