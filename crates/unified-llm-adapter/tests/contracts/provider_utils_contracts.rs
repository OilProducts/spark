use serde_json::json;
use unified_llm_adapter::SseParser;

#[test]
fn shared_sse_parser_handles_provider_stream_records_and_partial_chunks() {
    let mut parser = SseParser::default();

    assert!(parser
        .push_str(
            ": keepalive\r\nretry: 2500\nevent: legacy.sse.event\ndata: {\"type\":\"response.output_text.delta\","
        )
        .is_empty());
    assert!(parser.has_pending_input());

    let mut records = parser.push_str(
        "\ndata: \"delta\":\"Hel\"}\n\nevent: malformed.payload\ndata: {not-json}\n\ndata: [DONE]\n\n",
    );
    records.extend(parser.finish());

    assert_eq!(records.len(), 3);
    assert_eq!(
        records[0].event.as_deref(),
        Some("response.output_text.delta")
    );
    assert_eq!(records[0].sse_event.as_deref(), Some("legacy.sse.event"));
    assert_eq!(
        records[0].json_event.as_deref(),
        Some("response.output_text.delta")
    );
    assert_eq!(records[0].retry, Some(2500));
    assert_eq!(records[0].payload.as_ref().unwrap()["delta"], json!("Hel"));
    assert!(records[0].payload_error.is_none());
    assert_eq!(
        records[0].data,
        "{\"type\":\"response.output_text.delta\",\n\"delta\":\"Hel\"}"
    );

    assert_eq!(records[1].event.as_deref(), Some("malformed.payload"));
    assert!(records[1].payload.is_none());
    assert_eq!(records[1].payload_error.as_ref().unwrap().raw, "{not-json}");
    assert!(!records[1]
        .payload_error
        .as_ref()
        .unwrap()
        .message
        .is_empty());

    assert!(records[2].done);
    assert_eq!(records[2].data, "[DONE]");
    assert!(records[2].payload.is_none());
    assert!(records[2].payload_error.is_none());
}
