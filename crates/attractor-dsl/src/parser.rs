use std::collections::{BTreeMap, BTreeSet};

use attractor_core::{
    DotAttribute, DotEdge, DotGraph, DotNode, DotScopeDefaults, DotSubgraphScope, DotValue,
    DotValueType, DurationLiteral,
};

use crate::error::DotParseError;
use crate::lexer::{tokenize, Token, TokenKind};

const DURATION_UNITS: &[&str] = &["ms", "s", "m", "h", "d"];

pub fn parse_dot(source: &str) -> Result<DotGraph, DotParseError> {
    let tokens = tokenize(source)?;
    let mut parser = Parser { tokens, pos: 0 };
    let graph = parser.parse_graph()?;
    while parser.accept(TokenKind::Semi).is_some() {}
    let trailing = parser.current();
    let trailing_lower = if trailing.kind == TokenKind::Ident {
        trailing.value.to_ascii_lowercase()
    } else {
        String::new()
    };
    if matches!(trailing_lower.as_str(), "digraph" | "graph" | "strict") {
        return Err(DotParseError::new(
            "multiple graph declarations are not supported",
            trailing.line,
        ));
    }
    parser.expect(TokenKind::Eof, None)?;
    Ok(graph)
}

pub fn normalize_graph(graph: &DotGraph) -> DotGraph {
    let mut normalized = graph.clone();

    for attr in normalized.graph_attrs.values_mut() {
        attr.line = 0;
    }

    for node in normalized.nodes.values_mut() {
        node.line = 0;
        node.declaration_order = 0;
        for attr in node.attrs.values_mut() {
            attr.line = 0;
        }
    }

    for edge in &mut normalized.edges {
        edge.line = 0;
        for attr in edge.attrs.values_mut() {
            attr.line = 0;
        }
    }

    for attr in normalized.defaults.node.values_mut() {
        attr.line = 0;
    }
    for attr in normalized.defaults.edge.values_mut() {
        attr.line = 0;
    }

    fn normalize_subgraphs(subgraphs: &mut [DotSubgraphScope]) {
        for subgraph in subgraphs {
            for attr in subgraph.attrs.values_mut() {
                attr.line = 0;
            }
            for attr in subgraph.defaults.node.values_mut() {
                attr.line = 0;
            }
            for attr in subgraph.defaults.edge.values_mut() {
                attr.line = 0;
            }
            normalize_subgraphs(&mut subgraph.subgraphs);
        }
    }
    normalize_subgraphs(&mut normalized.subgraphs);

    normalized
}

#[derive(Debug, Clone, Default)]
struct Scope {
    node_defaults: BTreeMap<String, DotAttribute>,
    edge_defaults: BTreeMap<String, DotAttribute>,
}

impl Scope {
    fn child(&self) -> Self {
        Self {
            node_defaults: self.node_defaults.clone(),
            edge_defaults: self.edge_defaults.clone(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct SubgraphState {
    subgraph_id: Option<String>,
    attrs: BTreeMap<String, DotAttribute>,
    node_ids: Vec<String>,
    node_id_set: BTreeSet<String>,
    subgraphs: Vec<DotSubgraphScope>,
}

impl SubgraphState {
    fn new(subgraph_id: Option<String>) -> Self {
        Self {
            subgraph_id,
            ..Self::default()
        }
    }

    fn add_node_id(&mut self, node_id: &str) {
        if self.node_id_set.insert(node_id.to_string()) {
            self.node_ids.push(node_id.to_string());
        }
    }
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn current(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn peek(&self, n: usize) -> &Token {
        &self.tokens[self.pos + n]
    }

    fn advance(&mut self) -> Token {
        let token = self.current().clone();
        self.pos += 1;
        token
    }

    fn accept(&mut self, kind: TokenKind) -> Option<Token> {
        if self.current().kind != kind {
            return None;
        }
        Some(self.advance())
    }

    fn expect(&mut self, kind: TokenKind, value: Option<&str>) -> Result<Token, DotParseError> {
        let token = self.current();
        if token.kind != kind {
            return Err(DotParseError::new(
                format!("expected {}, got {}", kind.as_str(), token.kind.as_str()),
                token.line,
            ));
        }
        if let Some(expected_value) = value {
            if token.value != expected_value {
                return Err(DotParseError::new(
                    format!("expected {expected_value}, got {}", token.value),
                    token.line,
                ));
            }
        }
        Ok(self.advance())
    }

    fn parse_graph(&mut self) -> Result<DotGraph, DotParseError> {
        let first = self.current().clone();
        let first_lower = if first.kind == TokenKind::Ident {
            first.value.to_ascii_lowercase()
        } else {
            String::new()
        };
        match first_lower.as_str() {
            "strict" => {
                return Err(DotParseError::new(
                    "strict modifier is not supported",
                    first.line,
                ));
            }
            "graph" => {
                return Err(DotParseError::new(
                    "undirected graph declarations are not supported",
                    first.line,
                ));
            }
            "digraph" => {
                self.advance();
            }
            _ => {
                self.expect(TokenKind::Ident, Some("digraph"))?;
            }
        }

        let graph_id = self.expect(TokenKind::Ident, None)?;
        validate_identifier(&graph_id, "graph id")?;
        let mut graph = DotGraph {
            graph_id: graph_id.value,
            graph_attrs: BTreeMap::new(),
            nodes: BTreeMap::new(),
            edges: Vec::new(),
            defaults: DotScopeDefaults::default(),
            subgraphs: Vec::new(),
        };
        self.expect(TokenKind::LBrace, None)?;

        let mut scope = Scope::default();
        loop {
            while self.accept(TokenKind::Semi).is_some() {}
            if self.accept(TokenKind::RBrace).is_some() {
                break;
            }
            self.parse_statement(&mut graph, &mut scope, false, None)?;
            while self.accept(TokenKind::Semi).is_some() {}
        }

        graph.defaults = scope_defaults_from_scope(&scope);
        Ok(graph)
    }

    fn parse_statement(
        &mut self,
        graph: &mut DotGraph,
        scope: &mut Scope,
        in_subgraph: bool,
        mut subgraph_state: Option<&mut SubgraphState>,
    ) -> Result<(), DotParseError> {
        let token = self.current().clone();

        if token.kind == TokenKind::Ident && token.value == "subgraph" {
            self.advance();
            let subgraph_id = if self.current().kind == TokenKind::Ident {
                Some(self.advance().value)
            } else {
                None
            };
            self.expect(TokenKind::LBrace, None)?;
            let mut child_scope = scope.child();
            let mut child_subgraph = SubgraphState::new(subgraph_id);
            loop {
                while self.accept(TokenKind::Semi).is_some() {}
                if self.accept(TokenKind::RBrace).is_some() {
                    break;
                }
                self.parse_statement(graph, &mut child_scope, true, Some(&mut child_subgraph))?;
                while self.accept(TokenKind::Semi).is_some() {}
            }

            let label_attr = child_subgraph.attrs.get("label");
            let derived_class = derive_subgraph_class(label_attr.map(|attr| &attr.value));
            if !derived_class.is_empty() {
                let line = label_attr.map(|attr| attr.line).unwrap_or_default();
                for node_id in &child_subgraph.node_ids {
                    if let Some(node) = graph.nodes.get_mut(node_id) {
                        append_class(node, &derived_class, line);
                    }
                }
            }

            let subgraph_scope = DotSubgraphScope {
                id: child_subgraph.subgraph_id.clone(),
                attrs: child_subgraph.attrs.clone(),
                node_ids: child_subgraph.node_ids.clone(),
                defaults: scope_defaults_from_scope(&child_scope),
                subgraphs: child_subgraph.subgraphs.clone(),
            };

            if let Some(parent_state) = subgraph_state.as_deref_mut() {
                for node_id in &child_subgraph.node_ids {
                    parent_state.add_node_id(node_id);
                }
                parent_state.subgraphs.push(subgraph_scope);
            } else {
                graph.subgraphs.push(subgraph_scope);
            }
            return Ok(());
        }

        if token.kind == TokenKind::Ident
            && token.value == "graph"
            && self.peek(1).kind == TokenKind::LBracket
        {
            self.advance();
            let attrs = self.parse_attr_block()?;
            if !in_subgraph {
                graph.graph_attrs.extend(attrs);
            } else if let Some(state) = subgraph_state.as_deref_mut() {
                state.attrs.extend(attrs);
            }
            return Ok(());
        }

        if token.kind == TokenKind::Ident
            && token.value == "node"
            && self.peek(1).kind == TokenKind::LBracket
        {
            self.advance();
            scope.node_defaults.extend(self.parse_attr_block()?);
            return Ok(());
        }

        if token.kind == TokenKind::Ident
            && token.value == "edge"
            && self.peek(1).kind == TokenKind::LBracket
        {
            self.advance();
            scope.edge_defaults.extend(self.parse_attr_block()?);
            return Ok(());
        }

        if token.kind == TokenKind::Ident && self.peek(1).kind == TokenKind::Eq {
            let key = self.advance();
            validate_attr_key(&key)?;
            self.expect(TokenKind::Eq, None)?;
            let (value, value_type, line) = self.parse_value()?;
            let attr = DotAttribute {
                key: key.value.clone(),
                value,
                value_type,
                line,
            };
            if !in_subgraph {
                graph.graph_attrs.insert(key.value, attr);
            } else if let Some(state) = subgraph_state.as_deref_mut() {
                state.attrs.insert(key.value, attr);
            }
            return Ok(());
        }

        if token.kind == TokenKind::Ident {
            return self.parse_node_or_edge(graph, scope, subgraph_state);
        }

        Err(DotParseError::new(
            format!("unexpected token {}:{}", token.kind.as_str(), token.value),
            token.line,
        ))
    }

    fn parse_node_or_edge(
        &mut self,
        graph: &mut DotGraph,
        scope: &Scope,
        subgraph_state: Option<&mut SubgraphState>,
    ) -> Result<(), DotParseError> {
        let declaration_order = self.pos;
        let first = self.expect(TokenKind::Ident, None)?;
        validate_node_id(&first)?;
        self.reject_port_syntax_after_id()?;

        if self.accept(TokenKind::Arrow).is_some() {
            let mut chain_ids = vec![first];
            let next = self.expect(TokenKind::Ident, None)?;
            validate_node_id(&next)?;
            self.reject_port_syntax_after_id()?;
            chain_ids.push(next);

            while self.accept(TokenKind::Arrow).is_some() {
                let next = self.expect(TokenKind::Ident, None)?;
                validate_node_id(&next)?;
                self.reject_port_syntax_after_id()?;
                chain_ids.push(next);
            }

            let statement_attrs = if self.current().kind == TokenKind::LBracket {
                self.parse_attr_block()?
            } else {
                BTreeMap::new()
            };
            let mut effective = scope.edge_defaults.clone();
            effective.extend(statement_attrs);

            for pair in chain_ids.windows(2) {
                graph.edges.push(DotEdge {
                    source: pair[0].value.clone(),
                    target: pair[1].value.clone(),
                    attrs: effective.clone(),
                    line: pair[0].line,
                });
            }
            return Ok(());
        }

        let statement_attrs = if self.current().kind == TokenKind::LBracket {
            self.parse_attr_block()?
        } else {
            BTreeMap::new()
        };
        let mut effective = scope.node_defaults.clone();
        effective.extend(statement_attrs.clone());

        if let Some(existing) = graph.nodes.get_mut(&first.value) {
            let mut merged = existing.attrs.clone();
            for (key, attr) in &scope.node_defaults {
                merged.entry(key.clone()).or_insert_with(|| attr.clone());
            }
            merged.extend(statement_attrs.clone());
            existing.attrs = merged;
            existing
                .explicit_attr_keys
                .extend(statement_attrs.keys().cloned());
            if let Some(state) = subgraph_state {
                state.add_node_id(&first.value);
            }
            return Ok(());
        }

        graph.nodes.insert(
            first.value.clone(),
            DotNode {
                node_id: first.value.clone(),
                attrs: effective,
                line: first.line,
                declaration_order,
                explicit_attr_keys: statement_attrs.keys().cloned().collect(),
            },
        );
        if let Some(state) = subgraph_state {
            state.add_node_id(&first.value);
        }
        Ok(())
    }

    fn parse_attr_block(&mut self) -> Result<BTreeMap<String, DotAttribute>, DotParseError> {
        self.expect(TokenKind::LBracket, None)?;
        let mut attrs = BTreeMap::new();

        if self.accept(TokenKind::RBracket).is_some() {
            return Ok(attrs);
        }

        loop {
            let key = self.expect(TokenKind::Ident, None)?;
            validate_attr_key(&key)?;
            self.expect(TokenKind::Eq, None)?;
            let (mut value, value_type, line) = self.parse_value()?;
            if key.value == "class" && value_type == DotValueType::String {
                value = DotValue::String(normalize_class_list(&value.to_string()));
            }
            if key.value == "shape" && value_type == DotValueType::String {
                value = DotValue::String(normalize_shape(&value.to_string()));
            }
            attrs.insert(
                key.value.clone(),
                DotAttribute {
                    key: key.value,
                    value,
                    value_type,
                    line,
                },
            );

            if self.accept(TokenKind::RBracket).is_some() {
                break;
            }
            if self.accept(TokenKind::Comma).is_none() {
                let token = self.current();
                return Err(DotParseError::new(
                    "commas are required between attributes",
                    token.line,
                ));
            }
            if self.current().kind == TokenKind::RBracket {
                return Err(DotParseError::new(
                    "trailing comma is not allowed in attribute blocks",
                    self.current().line,
                ));
            }
        }

        Ok(attrs)
    }

    fn parse_value(&mut self) -> Result<(DotValue, DotValueType, usize), DotParseError> {
        let token = self.current().clone();

        if token.kind == TokenKind::Int
            && self.peek(1).kind == TokenKind::Ident
            && DURATION_UNITS.contains(&self.peek(1).value.as_str())
        {
            let int_token = self.advance();
            let unit_token = self.advance();
            let numeric_value = parse_i64(&int_token)?;
            let raw = format!("{numeric_value}{}", unit_token.value);
            return Ok((
                DotValue::Duration(DurationLiteral {
                    raw,
                    value: numeric_value,
                    unit: unit_token.value,
                }),
                DotValueType::Duration,
                int_token.line,
            ));
        }

        match token.kind {
            TokenKind::String => {
                let token = self.advance();
                Ok((
                    DotValue::String(token.value),
                    DotValueType::String,
                    token.line,
                ))
            }
            TokenKind::Int => {
                let token = self.advance();
                Ok((
                    DotValue::Integer(parse_i64(&token)?),
                    DotValueType::Integer,
                    token.line,
                ))
            }
            TokenKind::Float => {
                let token = self.advance();
                Ok((
                    DotValue::Float(parse_f64(&token)?),
                    DotValueType::Float,
                    token.line,
                ))
            }
            TokenKind::Ident => {
                let token = self.advance();
                if token.value == "true" {
                    return Ok((DotValue::Boolean(true), DotValueType::Boolean, token.line));
                }
                if token.value == "false" {
                    return Ok((DotValue::Boolean(false), DotValueType::Boolean, token.line));
                }
                let mut value_text = token.value;
                while self.accept(TokenKind::Colon).is_some() {
                    let suffix = self.expect(TokenKind::Ident, None)?;
                    value_text = format!("{value_text}:{}", suffix.value);
                }
                Ok((
                    DotValue::String(value_text),
                    DotValueType::String,
                    token.line,
                ))
            }
            _ => Err(DotParseError::new(
                format!(
                    "invalid value token {}:{}",
                    token.kind.as_str(),
                    token.value
                ),
                token.line,
            )),
        }
    }

    fn reject_port_syntax_after_id(&self) -> Result<(), DotParseError> {
        if self.current().kind == TokenKind::Colon {
            return Err(DotParseError::new(
                "port and compass point syntax is not supported",
                self.current().line,
            ));
        }
        Ok(())
    }
}

fn parse_i64(token: &Token) -> Result<i64, DotParseError> {
    token.value.parse::<i64>().map_err(|_| {
        DotParseError::new(
            format!("invalid integer literal '{}'", token.value),
            token.line,
        )
    })
}

fn parse_f64(token: &Token) -> Result<f64, DotParseError> {
    token.value.parse::<f64>().map_err(|_| {
        DotParseError::new(
            format!("invalid float literal '{}'", token.value),
            token.line,
        )
    })
}

fn validate_node_id(token: &Token) -> Result<(), DotParseError> {
    if is_node_id(&token.value) {
        return Ok(());
    }
    Err(DotParseError::new(
        format!(
            "invalid node id '{}', must match [A-Za-z_][A-Za-z0-9_]*",
            token.value
        ),
        token.line,
    ))
}

fn validate_identifier(token: &Token, kind: &str) -> Result<(), DotParseError> {
    if is_node_id(&token.value) {
        return Ok(());
    }
    Err(DotParseError::new(
        format!(
            "invalid {kind} '{}', must match [A-Za-z_][A-Za-z0-9_]*",
            token.value
        ),
        token.line,
    ))
}

fn validate_attr_key(token: &Token) -> Result<(), DotParseError> {
    if token.value.split('.').all(is_node_id) {
        return Ok(());
    }
    Err(DotParseError::new(
        format!("invalid attribute key '{}'", token.value),
        token.line,
    ))
}

fn is_node_id(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn scope_defaults_from_scope(scope: &Scope) -> DotScopeDefaults {
    DotScopeDefaults {
        node: scope.node_defaults.clone(),
        edge: scope.edge_defaults.clone(),
    }
}

fn derive_subgraph_class(label_value: Option<&DotValue>) -> String {
    let Some(label_value) = label_value else {
        return String::new();
    };

    let mut whitespace_normalized = String::new();
    let mut previous_whitespace = false;
    for ch in label_value.to_string().trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_whitespace() {
            if !previous_whitespace {
                whitespace_normalized.push('-');
                previous_whitespace = true;
            }
        } else {
            whitespace_normalized.push(ch);
            previous_whitespace = false;
        }
    }

    let filtered = whitespace_normalized
        .chars()
        .filter(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || *ch == '-');

    let mut collapsed = String::new();
    let mut previous_dash = false;
    for ch in filtered {
        if ch == '-' {
            if !previous_dash {
                collapsed.push(ch);
            }
            previous_dash = true;
        } else {
            collapsed.push(ch);
            previous_dash = false;
        }
    }

    collapsed.trim_matches('-').to_string()
}

fn append_class(node: &mut DotNode, class_name: &str, line: usize) {
    if let Some(existing) = node.attrs.get_mut("class") {
        let mut classes: Vec<String> = existing
            .value
            .to_string()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        if classes.iter().any(|existing| existing == class_name) {
            return;
        }
        classes.push(class_name.to_string());
        existing.value = DotValue::String(classes.join(","));
        existing.value_type = DotValueType::String;
        return;
    }

    node.attrs.insert(
        "class".to_string(),
        DotAttribute {
            key: "class".to_string(),
            value: DotValue::String(class_name.to_string()),
            value_type: DotValueType::String,
            line: if line == 0 { node.line } else { line },
        },
    );
}

fn normalize_class_list(raw: &str) -> String {
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for class_name in raw
        .split(',')
        .map(|value| value.trim().to_ascii_lowercase())
    {
        if class_name.is_empty() || seen.contains(&class_name) {
            continue;
        }
        seen.insert(class_name.clone());
        ordered.push(class_name);
    }
    ordered.join(",")
}

fn normalize_shape(raw: &str) -> String {
    let normalized = raw.trim();
    match normalized.to_ascii_lowercase().as_str() {
        "mdiamond" => "Mdiamond".to_string(),
        "msquare" => "Msquare".to_string(),
        "box" => "box".to_string(),
        "hexagon" => "hexagon".to_string(),
        "diamond" => "diamond".to_string(),
        "component" => "component".to_string(),
        "tripleoctagon" => "tripleoctagon".to_string(),
        "parallelogram" => "parallelogram".to_string(),
        "house" => "house".to_string(),
        _ => normalized.to_string(),
    }
}
