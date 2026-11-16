//! Typed form questions: the restricted schema an agent may ask with (ACP v1
//! form elicitation) and a person's reply. Chat, flow runs and the backends
//! check both here, so every boundary accepts exactly the same forms.
//!
//! A form is a flat object of string, number, integer, boolean and
//! multi-select fields. Any other keyword, type or string format is
//! unsupported, so a form never asks for something Spark cannot show or
//! check; secret inputs, such as passwords, are never offered.

use std::cmp::Ordering;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

/// A person's reply to a form: its content on accept, nothing otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum FormReply {
    Accept {
        /// Only the fields the person filled in: an omitted optional field
        /// stays omitted.
        #[serde(default, deserialize_with = "null_as_empty")]
        content: Map<String, Value>,
    },
    Decline,
    Cancel,
}

fn null_as_empty<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Map<String, Value>, D::Error> {
    Ok(Option::<Map<String, Value>>::deserialize(deserializer)?.unwrap_or_default())
}

const ANNOTATIONS: &[&str] = &["type", "title", "description", "_meta"];

/// Whether Spark can show `schema` and check replies to it; the reason when not.
pub fn check_schema(schema: &Value) -> Result<(), String> {
    let root = keywords(schema).ok_or("the form schema is not an object")?;
    if let Some(key) = root.keys().find(|key| {
        !ANNOTATIONS.contains(&key.as_str()) && !matches!(key.as_str(), "properties" | "required")
    }) {
        return Err(format!(
            "the form schema uses {key:?}, which Spark does not support"
        ));
    }
    if root.get("type").is_some_and(|kind| kind != "object") {
        return Err("the form schema is not an object schema".into());
    }
    let properties = properties(schema)?;
    for (name, property) in properties {
        check_property(name, property)?;
    }
    for name in required(schema)? {
        if !properties.contains_key(name) {
            return Err(format!(
                "the form requires {name:?}, which is not one of its fields"
            ));
        }
    }
    Ok(())
}

/// Whether `reply` answers a form of `schema`; the reason when not. Declining
/// and cancelling need no content; accepting needs every required field and
/// only valid values, and may be empty when nothing is required.
pub fn check_reply(schema: &Value, reply: &FormReply) -> Result<(), String> {
    check_schema(schema)?;
    let FormReply::Accept { content } = reply else {
        return Ok(());
    };
    let properties = properties(schema)?;
    for name in required(schema)? {
        if !content.contains_key(name) {
            return Err(format!("{} is required.", label(name, &properties[name])));
        }
    }
    for (name, value) in content {
        let property = properties
            .get(name)
            .ok_or_else(|| format!("{name:?} is not a field of this form."))?;
        check_value(&label(name, property), property, value)?;
    }
    Ok(())
}

/// The reply as transcript text: each answered field and its value.
pub fn reply_summary(schema: &Value, reply: &FormReply) -> String {
    let content = match reply {
        FormReply::Decline => return "Declined.".into(),
        FormReply::Cancel => return "Cancelled.".into(),
        FormReply::Accept { content } => content,
    };
    let properties = schema["properties"].as_object();
    let lines = content
        .iter()
        .map(|(name, value)| {
            let label = properties
                .and_then(|properties| properties.get(name))
                .map(|property| label(name, property))
                .unwrap_or_else(|| name.clone());
            let value = match value {
                Value::String(text) => text.clone(),
                Value::Array(items) => items
                    .iter()
                    .map(|item| {
                        item.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| item.to_string())
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                other => other.to_string(),
            };
            format!("{label}: {value}")
        })
        .collect::<Vec<_>>();
    match lines.is_empty() {
        true => "Accepted.".into(),
        false => format!("Accepted.\n{}", lines.join("\n")),
    }
}

fn properties(schema: &Value) -> Result<&Map<String, Value>, String> {
    static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
    match schema.get("properties") {
        None => Ok(EMPTY.get_or_init(Map::new)),
        Some(Value::Object(properties)) => Ok(properties),
        Some(_) => Err("the form's properties are not an object".into()),
    }
}

fn required(schema: &Value) -> Result<Vec<&str>, String> {
    match schema.get("required") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(names)) => names
            .iter()
            .map(|name| {
                name.as_str()
                    .ok_or_else(|| "the form's required fields are not names".to_string())
            })
            .collect(),
        Some(_) => Err("the form's required fields are not a list".into()),
    }
}

fn label(name: &str, property: &Value) -> String {
    property["title"]
        .as_str()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(name)
        .to_string()
}

/// A schema object's keywords. ACP defines a `null` keyword as omitted, so it drops out.
fn keywords(schema: &Value) -> Option<Map<String, Value>> {
    let object = schema.as_object()?;
    Some(
        object
            .iter()
            .filter(|(_, value)| !value.is_null())
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
}

fn check_property(name: &str, property: &Value) -> Result<(), String> {
    let object = &keywords(property).ok_or_else(|| format!("field {name:?} has no schema"))?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let supported: &[&str] = match kind {
        "string" => &[
            "minLength",
            "maxLength",
            "pattern",
            "format",
            "default",
            "enum",
            "oneOf",
        ],
        "number" | "integer" => &["minimum", "maximum", "default"],
        "boolean" => &["default"],
        "array" => &["minItems", "maxItems", "items", "default"],
        other => {
            return Err(format!(
                "field {name:?} has type {other:?}, which Spark does not support"
            ))
        }
    };
    if let Some(key) = object
        .keys()
        .find(|key| !ANNOTATIONS.contains(&key.as_str()) && !supported.contains(&key.as_str()))
    {
        return Err(format!(
            "field {name:?} uses {key:?}, which Spark does not support"
        ));
    }
    let malformed = |key: &str| format!("field {name:?} has a malformed {key:?}");
    let count = |key: &str| match object.get(key) {
        Some(value) if value.as_u64().is_none() => Err(malformed(key)),
        _ => Ok(()),
    };
    let number = |key: &str| match object.get(key) {
        Some(value) if !value.is_number() || (kind == "integer" && !is_integer(value)) => {
            Err(malformed(key))
        }
        _ => Ok(()),
    };
    match kind {
        "string" => {
            count("minLength")?;
            count("maxLength")?;
            if let Some(pattern) = object.get("pattern") {
                let pattern = pattern.as_str().ok_or_else(|| malformed("pattern"))?;
                regex::Regex::new(pattern).map_err(|_| {
                    format!("field {name:?} has pattern {pattern:?}, which Spark cannot check")
                })?;
            }
            if let Some(format) = object.get("format") {
                if !matches!(
                    format.as_str(),
                    Some("email" | "uri" | "date" | "date-time")
                ) {
                    return Err(format!(
                        "field {name:?} has format {format}, which Spark does not support"
                    ));
                }
            }
            if object.contains_key("enum") || object.contains_key("oneOf") {
                choices(object).ok_or_else(|| malformed("enum"))?;
                options_only(name, object.get("oneOf"))?;
            }
            if object
                .get("default")
                .is_some_and(|value| !value.is_string())
            {
                return Err(malformed("default"));
            }
        }
        "number" | "integer" => {
            number("minimum")?;
            number("maximum")?;
            number("default")?;
        }
        "boolean" => {
            if object
                .get("default")
                .is_some_and(|value| !value.is_boolean())
            {
                return Err(malformed("default"));
            }
        }
        _ => {
            count("minItems")?;
            count("maxItems")?;
            let items = object
                .get("items")
                .and_then(keywords)
                .ok_or_else(|| malformed("items"))?;
            item_choices(&items).ok_or_else(|| malformed("items"))?;
            let allowed: &[&str] = match items.contains_key("anyOf") {
                true => &["type", "anyOf", "_meta"],
                false => &["type", "enum", "_meta"],
            };
            if let Some(key) = items.keys().find(|key| !allowed.contains(&key.as_str())) {
                return Err(format!(
                    "field {name:?} uses {key:?} in its items, which Spark does not support"
                ));
            }
            options_only(name, items.get("anyOf"))?;
            if let Some(default) = object.get("default") {
                if !default
                    .as_array()
                    .is_some_and(|items| items.iter().all(Value::is_string))
                {
                    return Err(malformed("default"));
                }
            }
        }
    }
    Ok(())
}

/// Refuses a titled choice (`oneOf` or `anyOf` entry) with a keyword ACP's choices don't have.
fn options_only(name: &str, options: Option<&Value>) -> Result<(), String> {
    let mut keys = options
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .flat_map(Map::keys);
    match keys.find(|key| !matches!(key.as_str(), "const" | "title" | "description" | "_meta")) {
        Some(key) => Err(format!(
            "field {name:?} uses {key:?} in a choice, which Spark does not support"
        )),
        None => Ok(()),
    }
}

fn is_integer(value: &Value) -> bool {
    value.is_i64() || value.is_u64()
}

/// A single-choice string field's values: `enum`, or `oneOf`'s `const`s.
fn choices(property: &Map<String, Value>) -> Option<Vec<&str>> {
    match (property.get("enum"), property.get("oneOf")) {
        (Some(values), None) => values.as_array()?.iter().map(Value::as_str).collect(),
        (None, Some(options)) => options.as_array()?.iter().map(titled).collect(),
        _ => None,
    }
}

/// A multi-select field's values: `items.enum` of strings, or `items.anyOf`'s `const`s.
fn item_choices(items: &Map<String, Value>) -> Option<Vec<&str>> {
    match (items.get("enum"), items.get("anyOf")) {
        (Some(values), None) if items.get("type").is_some_and(|kind| kind == "string") => {
            values.as_array()?.iter().map(Value::as_str).collect()
        }
        (None, Some(options)) if items.get("type").is_none_or(|kind| kind == "string") => {
            options.as_array()?.iter().map(titled).collect()
        }
        _ => None,
    }
}

fn titled(option: &Value) -> Option<&str> {
    option["title"].as_str()?;
    option["const"].as_str()
}

fn check_value(label: &str, property: &Value, value: &Value) -> Result<(), String> {
    let object = &keywords(property).expect("a checked schema");
    let count = |key: &str| object.get(key).and_then(Value::as_u64);
    match object["type"].as_str().unwrap_or_default() {
        "string" => {
            let text = value
                .as_str()
                .ok_or_else(|| format!("{label} must be text."))?;
            if choices(object).is_some_and(|choices| !choices.contains(&text)) {
                return Err(format!("{label} must be one of its choices."));
            }
            let length = text.chars().count() as u64;
            if count("minLength").is_some_and(|min| length < min) {
                return Err(format!("{label} is too short."));
            }
            if count("maxLength").is_some_and(|max| length > max) {
                return Err(format!("{label} is too long."));
            }
            if let Some(pattern) = object.get("pattern").and_then(Value::as_str) {
                if !regex::Regex::new(pattern).is_ok_and(|pattern| pattern.is_match(text)) {
                    return Err(format!("{label} does not match the required pattern."));
                }
            }
            let valid = match object.get("format").and_then(Value::as_str) {
                Some("email") => is_email(text),
                Some("uri") => url::Url::parse(text).is_ok(),
                Some("date") => is_date(text),
                Some("date-time") => time::OffsetDateTime::parse(
                    text,
                    &time::format_description::well_known::Rfc3339,
                )
                .is_ok(),
                _ => true,
            };
            if !valid {
                return Err(format!(
                    "{label} must be a valid {}.",
                    object["format"].as_str().unwrap_or_default()
                ));
            }
        }
        kind @ ("number" | "integer") => {
            if !value.is_number() || (kind == "integer" && !is_integer(value)) {
                return Err(match kind {
                    "integer" => format!("{label} must be a whole number."),
                    _ => format!("{label} must be a number."),
                });
            }
            let beyond = |key: &str, side: Ordering| {
                object
                    .get(key)
                    .and_then(|bound| compare(value, bound))
                    .is_some_and(|order| order == side)
            };
            if beyond("minimum", Ordering::Less) {
                return Err(format!("{label} is below its minimum."));
            }
            if beyond("maximum", Ordering::Greater) {
                return Err(format!("{label} is above its maximum."));
            }
        }
        "boolean" => {
            if !value.is_boolean() {
                return Err(format!("{label} must be true or false."));
            }
        }
        _ => {
            let items = object.get("items").and_then(keywords);
            let choices =
                item_choices(items.as_ref().expect("a checked schema")).expect("a checked schema");
            let items = value
                .as_array()
                .filter(|items| {
                    items
                        .iter()
                        .all(|item| item.as_str().is_some_and(|item| choices.contains(&item)))
                })
                .ok_or_else(|| format!("{label} must list only its choices."))?;
            let length = items.len() as u64;
            if count("minItems").is_some_and(|min| length < min) {
                return Err(format!("{label} needs more choices."));
            }
            if count("maxItems").is_some_and(|max| length > max) {
                return Err(format!("{label} has too many choices."));
            }
        }
    }
    Ok(())
}

/// How a number compares with a bound, exactly: an integer beyond f64's precision is not
/// rounded into range. Rounding to f64 keeps order, so only a tie can hide a difference,
/// and only between whole numbers, which compare exactly.
fn compare(number: &Value, bound: &Value) -> Option<Ordering> {
    let whole = |value: &Value, float: f64| {
        (value.as_i64().map(i128::from))
            .or(value.as_u64().map(i128::from))
            .unwrap_or(float as i128)
    };
    let (a, b) = (number.as_f64()?, bound.as_f64()?);
    match a.partial_cmp(&b)? {
        Ordering::Equal => Some(whole(number, a).cmp(&whole(bound, b))),
        order => Some(order),
    }
}

fn is_email(text: &str) -> bool {
    !text.chars().any(char::is_whitespace)
        && matches!(text.split_once('@'), Some((local, domain))
            if !local.is_empty() && !domain.is_empty() && !domain.contains('@'))
}

fn is_date(text: &str) -> bool {
    let parts = text.split('-').collect::<Vec<_>>();
    let [year, month, day] = parts[..] else {
        return false;
    };
    if year.len() != 4
        || month.len() != 2
        || day.len() != 2
        || !text.chars().all(|c| c.is_ascii_digit() || c == '-')
    {
        return false;
    }
    let (Ok(year), Ok(month), Ok(day)) = (year.parse(), month.parse::<u8>(), day.parse()) else {
        return false;
    };
    time::Month::try_from(month)
        .is_ok_and(|month| time::Date::from_calendar_date(year, month, day).is_ok())
}
