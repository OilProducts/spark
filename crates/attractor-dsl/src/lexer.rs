use crate::DotParseError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenKind {
    Ident,
    String,
    Int,
    Float,
    Arrow,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Eq,
    Colon,
    Eof,
}

impl TokenKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ident => "IDENT",
            Self::String => "STRING",
            Self::Int => "INT",
            Self::Float => "FLOAT",
            Self::Arrow => "ARROW",
            Self::LBrace => "LBRACE",
            Self::RBrace => "RBRACE",
            Self::LBracket => "LBRACKET",
            Self::RBracket => "RBRACKET",
            Self::Comma => "COMMA",
            Self::Semi => "SEMI",
            Self::Eq => "EQ",
            Self::Colon => "COLON",
            Self::Eof => "EOF",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Token {
    pub(crate) kind: TokenKind,
    pub(crate) value: String,
    pub(crate) line: usize,
}

impl Token {
    fn new(kind: TokenKind, value: impl Into<String>, line: usize) -> Self {
        Self {
            kind,
            value: value.into(),
            line,
        }
    }
}

pub(crate) fn tokenize(source: &str) -> Result<Vec<Token>, DotParseError> {
    let stripped = strip_comments(source)?;
    let chars: Vec<char> = stripped.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    let mut line = 1;

    while i < chars.len() {
        let ch = chars[i];

        if matches!(ch, ' ' | '\t' | '\r') {
            i += 1;
            continue;
        }
        if ch == '\n' {
            line += 1;
            i += 1;
            continue;
        }

        if ch == '/' && chars.get(i + 1) == Some(&'/') {
            i += 2;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }

        if ch == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                if chars[i] == '\n' {
                    line += 1;
                }
                i += 1;
            }
            if i + 1 >= chars.len() {
                return Err(DotParseError::new("unterminated block comment", line));
            }
            i += 2;
            continue;
        }

        if ch == '-' && chars.get(i + 1) == Some(&'>') {
            tokens.push(Token::new(TokenKind::Arrow, "->", line));
            i += 2;
            continue;
        }

        if ch == '-' && chars.get(i + 1) == Some(&'-') {
            return Err(DotParseError::new(
                "undirected edges ('--') are not supported",
                line,
            ));
        }

        let punct_kind = match ch {
            '{' => Some(TokenKind::LBrace),
            '}' => Some(TokenKind::RBrace),
            '[' => Some(TokenKind::LBracket),
            ']' => Some(TokenKind::RBracket),
            ',' => Some(TokenKind::Comma),
            ';' => Some(TokenKind::Semi),
            '=' => Some(TokenKind::Eq),
            ':' => Some(TokenKind::Colon),
            _ => None,
        };
        if let Some(kind) = punct_kind {
            tokens.push(Token::new(kind, ch.to_string(), line));
            i += 1;
            continue;
        }

        if ch == '"' {
            let start_line = line;
            i += 1;
            let mut value = String::new();
            loop {
                let Some(c) = chars.get(i).copied() else {
                    return Err(DotParseError::new(
                        "unterminated string literal",
                        start_line,
                    ));
                };
                if c == '\\' {
                    let Some(escaped) = chars.get(i + 1).copied() else {
                        return Err(DotParseError::new(
                            "unterminated escape sequence",
                            start_line,
                        ));
                    };
                    let mapped = match escaped {
                        '"' => '"',
                        'n' => '\n',
                        't' => '\t',
                        '\\' => '\\',
                        other => {
                            return Err(DotParseError::new(
                                format!("unsupported escape \\{other}"),
                                line,
                            ));
                        }
                    };
                    value.push(mapped);
                    i += 2;
                    continue;
                }
                if c == '"' {
                    i += 1;
                    break;
                }
                if c == '\n' {
                    return Err(DotParseError::new(
                        "unescaped newline in string literal",
                        line,
                    ));
                }
                value.push(c);
                i += 1;
            }
            tokens.push(Token::new(TokenKind::String, value, start_line));
            continue;
        }

        let starts_number = ch.is_ascii_digit();
        let starts_signed_number = matches!(ch, '+' | '-')
            && chars
                .get(i + 1)
                .is_some_and(|next| next.is_ascii_digit() || *next == '.')
            && (chars.get(i + 1).is_some_and(char::is_ascii_digit)
                || chars.get(i + 2).is_some_and(char::is_ascii_digit));
        let starts_leading_dot_float =
            ch == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit);

        if starts_number || starts_signed_number || starts_leading_dot_float {
            let start = i;
            let start_line = line;
            if matches!(chars[i], '+' | '-') {
                i += 1;
            }

            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let mut is_float = false;
            if i < chars.len() && chars[i] == '.' {
                is_float = true;
                i += 1;
                if i >= chars.len() || !chars[i].is_ascii_digit() {
                    return Err(DotParseError::new("invalid float literal", start_line));
                }
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }

            if i < chars.len() && matches!(chars[i], 'e' | 'E') {
                is_float = true;
                i += 1;
                if i < chars.len() && matches!(chars[i], '+' | '-') {
                    i += 1;
                }
                if i >= chars.len() || !chars[i].is_ascii_digit() {
                    return Err(DotParseError::new("invalid float literal", start_line));
                }
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }

            tokens.push(Token::new(
                if is_float {
                    TokenKind::Float
                } else {
                    TokenKind::Int
                },
                chars[start..i].iter().collect::<String>(),
                start_line,
            ));
            continue;
        }

        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = i;
            i += 1;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric()
                    || chars[i] == '_'
                    || chars[i] == '.'
                    || chars[i] == '-')
            {
                i += 1;
            }
            tokens.push(Token::new(
                TokenKind::Ident,
                chars[start..i].iter().collect::<String>(),
                line,
            ));
            continue;
        }

        if ch == '<' {
            return Err(DotParseError::new(
                "HTML-like labels are not supported",
                line,
            ));
        }

        return Err(DotParseError::new(
            format!("unexpected character '{ch}'"),
            line,
        ));
    }

    tokens.push(Token::new(TokenKind::Eof, "", line));
    Ok(tokens)
}

fn strip_comments(source: &str) -> Result<String, DotParseError> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut line = 1;
    let mut in_string = false;

    while i < chars.len() {
        let ch = chars[i];

        if in_string {
            out.push(ch);
            if ch == '\\' {
                let Some(next) = chars.get(i + 1).copied() else {
                    return Err(DotParseError::new("unterminated escape sequence", line));
                };
                out.push(next);
                i += 2;
                continue;
            }
            if ch == '"' {
                in_string = false;
            }
            if ch == '\n' {
                line += 1;
            }
            i += 1;
            continue;
        }

        if ch == '"' {
            in_string = true;
            out.push(ch);
            i += 1;
            continue;
        }

        if ch == '/' && chars.get(i + 1) == Some(&'/') {
            i += 2;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }

        if ch == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                if chars[i] == '\n' {
                    line += 1;
                    out.push('\n');
                }
                i += 1;
            }
            if i + 1 >= chars.len() {
                return Err(DotParseError::new("unterminated block comment", line));
            }
            i += 2;
            continue;
        }

        if ch == '\n' {
            line += 1;
        }
        out.push(ch);
        i += 1;
    }

    Ok(out)
}
