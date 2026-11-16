//! Typed form questions: which restricted schemas Spark accepts, and which
//! replies answer them, as every boundary checks them.

use serde_json::{json, Value};
use spark_common::forms::{check_reply, check_schema, reply_summary, FormReply};

fn schema() -> Value {
    json!({
        "type": "object",
        "title": "Details",
        "properties": {
            "name": {"type": "string", "title": "Name", "minLength": 2, "maxLength": 5},
            "code": {"type": "string", "pattern": "^[A-Z]+$"},
            "email": {"type": "string", "format": "email"},
            "site": {"type": "string", "format": "uri"},
            "day": {"type": "string", "format": "date"},
            "at": {"type": "string", "format": "date-time"},
            "color": {"type": "string", "enum": ["red", "green"], "default": "red"},
            "size": {"type": "string", "oneOf": [
                {"const": "s", "title": "Small"},
                {"const": "l", "title": "Large"},
            ]},
            "count": {"type": "integer", "minimum": 0, "maximum": 3},
            "ratio": {"type": "number", "minimum": -1.5, "maximum": 1.5},
            "agree": {"type": "boolean", "default": true},
            "tags": {"type": "array", "items": {"type": "string", "enum": ["a", "b", "c"]}, "maxItems": 2},
            "picks": {"type": "array", "minItems": 1, "items": {"anyOf": [{"const": "x", "title": "Ex"}]}},
        },
        "required": ["name", "count", "agree"],
    })
}

fn accept(content: Value) -> FormReply {
    serde_json::from_value(json!({"action": "accept", "content": content})).unwrap()
}

#[test]
fn every_restricted_field_type_is_supported() {
    assert_eq!(check_schema(&schema()), Ok(()));
    let full = accept(json!({
        "name": "Ann", "code": "AB", "email": "a@b.co", "site": "https://x.test/p",
        "day": "2024-02-29", "at": "2024-02-29T10:00:00Z", "color": "green", "size": "l",
        "count": 3, "ratio": -1.5, "agree": true, "tags": ["a", "c"], "picks": ["x"],
    }));
    assert_eq!(check_reply(&schema(), &full), Ok(()));
}

#[test]
fn zero_false_and_omitted_optional_fields_are_valid_answers() {
    let reply = accept(json!({"name": "Ann", "count": 0, "agree": false, "tags": []}));
    assert_eq!(check_reply(&schema(), &reply), Ok(()));
    assert_eq!(
        serde_json::to_value(&reply).unwrap(),
        json!({"action": "accept", "content": {"name": "Ann", "count": 0, "agree": false, "tags": []}}),
        "values keep their types and omitted fields stay omitted"
    );
    let open = json!({"type": "object", "properties": {"note": {"type": "string"}}});
    assert_eq!(check_reply(&open, &accept(json!({}))), Ok(()));
    let bare: FormReply = serde_json::from_value(json!({"action": "accept"})).unwrap();
    assert_eq!(bare, accept(json!({})), "accept may carry no content");
    assert_eq!(check_reply(&open, &bare), Ok(()));
    let null: FormReply =
        serde_json::from_value(json!({"action": "accept", "content": null})).unwrap();
    assert_eq!(null, bare);
}

#[test]
fn decline_and_cancel_need_no_answers() {
    for (reply, wire) in [
        (FormReply::Decline, json!({"action": "decline"})),
        (FormReply::Cancel, json!({"action": "cancel"})),
    ] {
        assert_eq!(check_reply(&schema(), &reply), Ok(()));
        assert_eq!(serde_json::to_value(&reply).unwrap(), wire);
        assert_eq!(serde_json::from_value::<FormReply>(wire).unwrap(), reply);
    }
    assert!(serde_json::from_value::<FormReply>(json!({"action": "maybe"})).is_err());
}

#[test]
fn invalid_answers_are_refused_with_the_field_named() {
    let base = json!({"name": "Ann", "count": 1, "agree": true});
    for (field, value, reason) in [
        ("name", json!("A"), "Name is too short."),
        ("name", json!("Annabel"), "Name is too long."),
        ("name", json!(7), "Name must be text."),
        (
            "code",
            json!("ab"),
            "code does not match the required pattern.",
        ),
        (
            "email",
            json!("not an email"),
            "email must be a valid email.",
        ),
        ("site", json!("no scheme"), "site must be a valid uri."),
        ("day", json!("2023-02-29"), "day must be a valid date."),
        (
            "at",
            json!("2024-02-29 10:00"),
            "at must be a valid date-time.",
        ),
        ("color", json!("blue"), "color must be one of its choices."),
        ("size", json!("Small"), "size must be one of its choices."),
        ("count", json!(1.5), "count must be a whole number."),
        ("count", json!(4), "count is above its maximum."),
        ("count", json!(-1), "count is below its minimum."),
        ("count", json!("1"), "count must be a whole number."),
        ("ratio", json!(true), "ratio must be a number."),
        ("agree", json!("yes"), "agree must be true or false."),
        ("tags", json!(["d"]), "tags must list only its choices."),
        ("tags", json!(["a", "b", "c"]), "tags has too many choices."),
        ("picks", json!([]), "picks needs more choices."),
        ("other", json!(1), "\"other\" is not a field of this form."),
    ] {
        let mut content = base.clone();
        content[field] = value;
        assert_eq!(
            check_reply(&schema(), &accept(content)),
            Err(reason.to_string()),
            "{field}"
        );
    }
    assert_eq!(
        check_reply(&schema(), &accept(json!({"name": "Ann", "agree": true}))),
        Err("count is required.".to_string())
    );
}

#[test]
fn unsupported_schemas_fail_explicitly() {
    let field = |property: Value| json!({"type": "object", "properties": {"f": property}});
    for (schema, reason) in [
        (json!([]), "the form schema is not an object"),
        (
            json!({"type": "array"}),
            "the form schema is not an object schema",
        ),
        (
            json!({"type": "object", "additionalProperties": false}),
            "\"additionalProperties\"",
        ),
        (field(json!({"type": "object"})), "type \"object\""),
        (
            field(json!({"type": "string", "format": "password"})),
            "format \"password\"",
        ),
        (
            field(json!({"type": "string", "pattern": "(?=a)"})),
            "cannot check",
        ),
        (field(json!({"type": "string", "$ref": "#/x"})), "\"$ref\""),
        (
            field(json!({"type": "array", "items": {"type": "number"}})),
            "malformed \"items\"",
        ),
        (
            field(json!({"type": "integer", "default": 1.5})),
            "malformed \"default\"",
        ),
        (
            field(json!({"type": "boolean", "default": "yes"})),
            "malformed \"default\"",
        ),
        (
            json!({"type": "object", "properties": {}, "required": ["f"]}),
            "not one of its fields",
        ),
    ] {
        let error = check_schema(&schema).expect_err(&schema.to_string());
        assert!(error.contains(reason), "{error} for {schema}");
        assert!(
            check_reply(&schema, &FormReply::Decline).is_err(),
            "no reply answers {schema}"
        );
    }
}

#[test]
fn a_reply_reads_as_its_labelled_values() {
    let reply = accept(json!({"name": "Ann", "count": 0, "agree": false, "tags": ["a", "b"]}));
    assert_eq!(
        reply_summary(&schema(), &reply),
        "Accepted.\nName: Ann\ncount: 0\nagree: false\ntags: a, b"
    );
    assert_eq!(reply_summary(&schema(), &accept(json!({}))), "Accepted.");
    assert_eq!(reply_summary(&schema(), &FormReply::Decline), "Declined.");
    assert_eq!(reply_summary(&schema(), &FormReply::Cancel), "Cancelled.");
}
