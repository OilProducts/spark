use crate::context::{context_value_to_string, AttractorContext};
use crate::outcomes::Outcome;

pub fn evaluate_condition(condition: &str, outcome: &Outcome, context: &AttractorContext) -> bool {
    let text = condition.trim();
    if text.is_empty() {
        return true;
    }

    for clause in split_condition_clauses(text) {
        let clause = clause.trim();
        if clause.is_empty() {
            continue;
        }

        if let Some((key, op, expected)) = parse_comparison_clause(clause) {
            let expected = normalize_condition_literal(expected);
            let actual = resolve_condition_key(key, outcome, context);
            if op == "=" && actual != expected {
                return false;
            }
            if op == "!=" && actual == expected {
                return false;
            }
            continue;
        }

        if !is_bare_condition_key(clause)
            || resolve_condition_key(clause, outcome, context).is_empty()
        {
            return false;
        }
    }

    true
}

pub fn split_condition_clauses(condition: &str) -> Vec<String> {
    let mut clauses = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    let mut chars = condition.chars().peekable();

    while let Some(ch) = chars.next() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            current.push(ch);
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_quotes = !in_quotes;
            current.push(ch);
            continue;
        }
        if !in_quotes && ch == '&' && chars.peek().copied() == Some('&') {
            chars.next();
            clauses.push(std::mem::take(&mut current));
            continue;
        }
        current.push(ch);
    }

    clauses.push(current);
    clauses
}

pub fn normalize_condition_literal(raw: &str) -> String {
    let text = raw.trim();
    if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
        return unescape_quoted_literal(&text[1..text.len() - 1]);
    }
    text.to_string()
}

fn parse_comparison_clause(clause: &str) -> Option<(&str, &'static str, &str)> {
    let mut key_end = 0;
    for (index, ch) in clause.char_indices() {
        let valid = if index == 0 {
            ch.is_ascii_alphabetic() || ch == '_'
        } else {
            ch.is_ascii_alphanumeric() || ch == '_' || ch == '.'
        };
        if !valid {
            break;
        }
        key_end = index + ch.len_utf8();
    }

    if key_end == 0 {
        return None;
    }

    let key = &clause[..key_end];
    let rest = clause[key_end..].trim_start();
    if let Some(expected) = rest.strip_prefix("!=") {
        if expected.is_empty() {
            return None;
        }
        return Some((key, "!=", expected));
    }
    if let Some(expected) = rest.strip_prefix('=') {
        if expected.is_empty() {
            return None;
        }
        return Some((key, "=", expected));
    }
    None
}

fn is_bare_condition_key(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')
}

fn resolve_condition_key(key: &str, outcome: &Outcome, context: &AttractorContext) -> String {
    if key == "outcome" {
        return outcome.status.as_str().to_string();
    }
    if key == "preferred_label" {
        return outcome.preferred_label.clone();
    }
    if let Some(unprefixed_key) = key.strip_prefix("context.") {
        if let Some(value) = context.get(key) {
            return context_value_to_string(value);
        }
        if let Some(value) = context.get(unprefixed_key) {
            return context_value_to_string(value);
        }
        return context.get_context_path(unprefixed_key);
    }
    if let Some(value) = context.get(key) {
        return context_value_to_string(value);
    }
    String::new()
}

fn unescape_quoted_literal(text: &str) -> String {
    let mut unescaped = String::new();
    let mut escaped = false;
    for ch in text.chars() {
        if escaped {
            if matches!(ch, '"' | '\\') {
                unescaped.push(ch);
            } else {
                unescaped.push('\\');
                unescaped.push(ch);
            }
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        unescaped.push(ch);
    }
    if escaped {
        unescaped.push('\\');
    }
    unescaped
}
