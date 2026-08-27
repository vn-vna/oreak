use std::collections::HashSet;

use thiserror::Error;

const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug)]
pub(crate) struct JsonNode {
    raw: Option<String>,
    kind: JsonKind,
}

#[derive(Clone, Debug)]
enum JsonKind {
    Null,
    Bool(bool),
    Number,
    String(String),
    Array(Vec<JsonNode>),
    Object(Vec<JsonMember>),
}

#[derive(Clone, Debug)]
struct JsonMember {
    name: String,
    raw_name: String,
    value: JsonNode,
}

impl JsonNode {
    pub(crate) fn parse_root(input: &str) -> Result<Self, JsonError> {
        let mut parser = Parser {
            input,
            bytes: input.as_bytes(),
            offset: 0,
        };
        parser.skip_whitespace();
        let value = parser.parse_value(0)?;
        parser.skip_whitespace();
        if parser.offset != parser.bytes.len() {
            return Err(parser.error("additional JSON content follows the root value"));
        }
        if !matches!(value.kind, JsonKind::Object(_)) {
            return Err(JsonError::new(0, "the level JSON root must be an object"));
        }
        Ok(value)
    }

    pub(crate) fn render(&self) -> String {
        if let Some(raw) = &self.raw {
            return raw.clone();
        }
        match &self.kind {
            JsonKind::Null => "null".to_owned(),
            JsonKind::Bool(value) => value.to_string(),
            JsonKind::Number => unreachable!("generated numbers always retain canonical raw text"),
            JsonKind::String(value) => serde_json::to_string(value).expect("strings serialize"),
            JsonKind::Array(values) => {
                let rendered: Vec<_> = values.iter().map(Self::render).collect();
                format!("[{}]", rendered.join(","))
            }
            JsonKind::Object(members) => {
                let rendered: Vec<_> = members
                    .iter()
                    .map(|member| format!("{}:{}", member.raw_name, member.value.render()))
                    .collect();
                format!("{{{}}}", rendered.join(","))
            }
        }
    }

    pub(crate) fn null() -> Self {
        Self {
            raw: None,
            kind: JsonKind::Null,
        }
    }

    pub(crate) fn bool(value: bool) -> Self {
        Self {
            raw: None,
            kind: JsonKind::Bool(value),
        }
    }

    pub(crate) fn number(value: impl ToString) -> Self {
        Self {
            raw: Some(value.to_string()),
            kind: JsonKind::Number,
        }
    }

    pub(crate) fn string(value: impl Into<String>) -> Self {
        Self {
            raw: None,
            kind: JsonKind::String(value.into()),
        }
    }

    pub(crate) fn array(values: Vec<Self>) -> Self {
        Self {
            raw: None,
            kind: JsonKind::Array(values),
        }
    }

    pub(crate) fn object(values: Vec<(&str, Self)>) -> Self {
        Self {
            raw: None,
            kind: JsonKind::Object(
                values
                    .into_iter()
                    .map(|(name, value)| JsonMember {
                        name: name.to_owned(),
                        raw_name: serde_json::to_string(name).expect("property names serialize"),
                        value,
                    })
                    .collect(),
            ),
        }
    }

    pub(crate) fn as_object(&self) -> Option<&Self> {
        matches!(self.kind, JsonKind::Object(_)).then_some(self)
    }

    pub(crate) fn get(&self, name: &str) -> Option<&Self> {
        let JsonKind::Object(members) = &self.kind else {
            return None;
        };
        members
            .iter()
            .find(|member| member.name == name)
            .map(|member| &member.value)
    }

    pub(crate) fn set(&self, name: &str, value: Self) -> Result<Self, JsonError> {
        let JsonKind::Object(source) = &self.kind else {
            return Err(JsonError::new(0, "value is not a JSON object"));
        };
        let mut members = source.clone();
        if let Some(member) = members.iter_mut().find(|member| member.name == name) {
            member.value = value;
        } else {
            members.push(JsonMember {
                name: name.to_owned(),
                raw_name: serde_json::to_string(name).expect("property names serialize"),
                value,
            });
        }
        Ok(Self {
            raw: None,
            kind: JsonKind::Object(members),
        })
    }

    pub(crate) fn as_array(&self) -> Option<&[Self]> {
        let JsonKind::Array(values) = &self.kind else {
            return None;
        };
        Some(values)
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        let JsonKind::String(value) = &self.kind else {
            return None;
        };
        Some(value)
    }

    pub(crate) fn as_bool(&self) -> Option<bool> {
        let JsonKind::Bool(value) = self.kind else {
            return None;
        };
        Some(value)
    }

    pub(crate) fn is_null(&self) -> bool {
        matches!(self.kind, JsonKind::Null)
    }

    pub(crate) fn as_i64(&self) -> Option<i64> {
        matches!(self.kind, JsonKind::Number)
            .then(|| self.raw.as_deref())
            .flatten()
            .and_then(|raw| serde_json::from_str::<i64>(raw).ok())
    }

    pub(crate) fn as_f64(&self) -> Option<f64> {
        matches!(self.kind, JsonKind::Number)
            .then(|| self.raw.as_deref())
            .flatten()
            .and_then(|raw| serde_json::from_str::<f64>(raw).ok())
    }

    pub(crate) fn to_serde_value(&self) -> serde_json::Value {
        serde_json::from_str(&self.render()).expect("validated JSON node must deserialize")
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("invalid JSON at byte {offset}: {message}")]
pub(crate) struct JsonError {
    pub(crate) offset: usize,
    pub(crate) message: String,
}

impl JsonError {
    fn new(offset: usize, message: impl Into<String>) -> Self {
        Self {
            offset,
            message: message.into(),
        }
    }
}

struct Parser<'a> {
    input: &'a str,
    bytes: &'a [u8],
    offset: usize,
}

impl Parser<'_> {
    fn parse_value(&mut self, depth: usize) -> Result<JsonNode, JsonError> {
        if depth > MAX_DEPTH {
            return Err(self.error("JSON nesting exceeds 64 levels"));
        }
        self.skip_whitespace();
        let start = self.offset;
        let kind = match self.peek() {
            Some(b'n') => {
                self.consume_literal(b"null")?;
                JsonKind::Null
            }
            Some(b't') => {
                self.consume_literal(b"true")?;
                JsonKind::Bool(true)
            }
            Some(b'f') => {
                self.consume_literal(b"false")?;
                JsonKind::Bool(false)
            }
            Some(b'"') => JsonKind::String(self.parse_string()?.0),
            Some(b'[') => JsonKind::Array(self.parse_array(depth + 1)?),
            Some(b'{') => JsonKind::Object(self.parse_object(depth + 1)?),
            Some(b'-' | b'0'..=b'9') => {
                self.parse_number()?;
                JsonKind::Number
            }
            Some(_) => return Err(self.error("expected a JSON value")),
            None => return Err(self.error("unexpected end of JSON input")),
        };
        Ok(JsonNode {
            raw: Some(self.input[start..self.offset].to_owned()),
            kind,
        })
    }

    fn parse_array(&mut self, depth: usize) -> Result<Vec<JsonNode>, JsonError> {
        self.offset += 1;
        self.skip_whitespace();
        let mut values = Vec::new();
        if self.take(b']') {
            return Ok(values);
        }
        loop {
            values.push(self.parse_value(depth)?);
            self.skip_whitespace();
            if self.take(b']') {
                return Ok(values);
            }
            if !self.take(b',') {
                return Err(self.error("expected ',' or ']' in array"));
            }
            self.skip_whitespace();
            if self.peek() == Some(b']') {
                return Err(self.error("trailing commas are not valid JSON"));
            }
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<Vec<JsonMember>, JsonError> {
        self.offset += 1;
        self.skip_whitespace();
        let mut members = Vec::new();
        let mut names = HashSet::new();
        if self.take(b'}') {
            return Ok(members);
        }
        loop {
            if self.peek() != Some(b'"') {
                return Err(self.error("object property names must be JSON strings"));
            }
            let (name, raw_name) = self.parse_string()?;
            if !names.insert(name.clone()) {
                return Err(self.error(format!("duplicate object property '{name}'")));
            }
            self.skip_whitespace();
            if !self.take(b':') {
                return Err(self.error("expected ':' after object property name"));
            }
            let value = self.parse_value(depth)?;
            members.push(JsonMember {
                name,
                raw_name,
                value,
            });
            self.skip_whitespace();
            if self.take(b'}') {
                return Ok(members);
            }
            if !self.take(b',') {
                return Err(self.error("expected ',' or '}' in object"));
            }
            self.skip_whitespace();
            if self.peek() == Some(b'}') {
                return Err(self.error("trailing commas are not valid JSON"));
            }
        }
    }

    fn parse_string(&mut self) -> Result<(String, String), JsonError> {
        let start = self.offset;
        self.offset += 1;
        let mut escaped = false;
        while let Some(byte) = self.peek() {
            if byte < 0x20 {
                return Err(self.error("unescaped control character in JSON string"));
            }
            self.offset += 1;
            if escaped {
                escaped = false;
                continue;
            }
            match byte {
                b'\\' => escaped = true,
                b'"' => {
                    let raw = &self.input[start..self.offset];
                    let value = serde_json::from_str(raw)
                        .map_err(|error| self.error(format!("invalid JSON string: {error}")))?;
                    return Ok((value, raw.to_owned()));
                }
                _ => {}
            }
        }
        Err(self.error("unterminated JSON string"))
    }

    fn parse_number(&mut self) -> Result<(), JsonError> {
        let start = self.offset;
        self.take(b'-');
        match self.peek() {
            Some(b'0') => {
                self.offset += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(self.error("leading zero in JSON number"));
                }
            }
            Some(b'1'..=b'9') => self.consume_digits(),
            _ => return Err(self.error("invalid JSON number")),
        }
        if self.take(b'.') {
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error("fraction requires at least one digit"));
            }
            self.consume_digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.offset += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.offset += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error("exponent requires at least one digit"));
            }
            self.consume_digits();
        }
        serde_json::from_str::<serde_json::Number>(&self.input[start..self.offset])
            .map_err(|error| self.error(format!("invalid JSON number: {error}")))?;
        Ok(())
    }

    fn consume_digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.offset += 1;
        }
    }

    fn consume_literal(&mut self, literal: &[u8]) -> Result<(), JsonError> {
        if self.bytes.get(self.offset..self.offset + literal.len()) != Some(literal) {
            return Err(self.error("invalid JSON literal"));
        }
        self.offset += literal.len();
        Ok(())
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.offset += 1;
        }
    }

    fn take(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.offset += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    fn error(&self, message: impl Into<String>) -> JsonError {
        JsonError::new(self.offset, message)
    }
}
