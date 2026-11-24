use std::fs::{self, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::ipc::json_string;
use crate::splits::Axis;

pub const VERSION: u32 = 1;
const MIN_RATIO: f32 = 0.15;
const MAX_RATIO: f32 = 0.85;

#[derive(Debug, Clone, PartialEq)]
pub struct SavedSession {
    pub version: u32,
    pub name: String,
    pub active_tab: usize,
    pub tabs: Vec<SavedTab>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SavedTab {
    pub title: Option<String>,
    pub layout: SavedNode,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SavedNode {
    Pane {
        cwd: Option<String>,
        active: bool,
    },
    Split {
        axis: Axis,
        ratio: f32,
        first: Box<SavedNode>,
        second: Box<SavedNode>,
    },
}

pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 {
        return Err("session name must be 1 to 64 characters".to_string());
    }
    if name == "." || name == ".." {
        return Err(format!("invalid session name '{name}'"));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(format!(
            "invalid session name '{name}': only [a-zA-Z0-9._-] allowed"
        ));
    }
    Ok(())
}

pub fn dir() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|home| PathBuf::from(home).join(".local/state"))
        })
        .map(|state| state.join("koral/sessions"))
}

pub fn save(dir: &Path, session: &SavedSession) -> Result<PathBuf, String> {
    validate_name(&session.name)?;
    if session.version != VERSION {
        return Err(format!(
            "unsupported session schema version {}",
            session.version
        ));
    }
    fs::create_dir_all(dir).map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
    let _ = fs::set_permissions(dir, Permissions::from_mode(0o700));

    let payload = encode_json(session);
    let target = dir.join(format!("{}.json", session.name));
    let tmp = dir.join(format!(".{}.json.tmp.{}", session.name, std::process::id()));

    if let Err(err) = fs::write(&tmp, payload.as_bytes()) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("cannot write {}: {err}", tmp.display()));
    }
    if let Err(err) = fs::set_permissions(&tmp, Permissions::from_mode(0o600)) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("cannot secure {}: {err}", tmp.display()));
    }
    if let Err(err) = fs::rename(&tmp, &target) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("cannot rename {}: {err}", target.display()));
    }
    Ok(target)
}

pub fn load(dir: &Path, name: &str) -> Result<SavedSession, String> {
    validate_name(name)?;
    let target = dir.join(format!("{name}.json"));
    let text = fs::read_to_string(&target)
        .map_err(|err| format!("cannot read session '{name}': {err}"))?;
    decode_json(&text)
}

pub fn list(dir: &Path) -> Result<Vec<String>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let entries =
        fs::read_dir(dir).map_err(|err| format!("cannot list {}: {err}", dir.display()))?;
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            && validate_name(stem).is_ok()
        {
            names.push(stem.to_string());
        }
    }
    names.sort();
    Ok(names)
}

pub fn render_sessions_text(names: &[String]) -> String {
    let mut out = String::new();
    for name in names {
        out.push_str(name);
        out.push('\n');
    }
    out
}

pub fn render_sessions_json(names: &[String]) -> String {
    let mut out = String::from("{\"version\":1,\"sessions\":[");
    for (idx, name) in names.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        out.push_str(&json_string(name));
    }
    out.push_str("]}");
    out
}

pub fn encode_json(session: &SavedSession) -> String {
    let mut out = String::from("{\"version\":");
    out.push_str(&session.version.to_string());
    out.push_str(",\"name\":");
    out.push_str(&json_string(&session.name));
    out.push_str(",\"active_tab\":");
    out.push_str(&session.active_tab.to_string());
    out.push_str(",\"tabs\":[");
    for (idx, tab) in session.tabs.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        out.push_str("{\"title\":");
        match &tab.title {
            Some(title) => out.push_str(&json_string(title)),
            None => out.push_str("null"),
        }
        out.push_str(",\"layout\":");
        encode_node(&tab.layout, &mut out);
        out.push('}');
    }
    out.push_str("]}");
    out
}

fn encode_node(node: &SavedNode, out: &mut String) {
    match node {
        SavedNode::Pane { cwd, active } => {
            out.push_str("{\"kind\":\"pane\",\"cwd\":");
            match cwd {
                Some(path) => out.push_str(&json_string(path)),
                None => out.push_str("null"),
            }
            out.push_str(",\"active\":");
            out.push_str(if *active { "true" } else { "false" });
            out.push('}');
        }
        SavedNode::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            out.push_str("{\"kind\":\"split\",\"axis\":");
            out.push_str(match axis {
                Axis::Horizontal => "\"horizontal\"",
                Axis::Vertical => "\"vertical\"",
            });
            out.push_str(",\"ratio\":");
            let clamped = ratio.clamp(MIN_RATIO, MAX_RATIO);
            out.push_str(&format!("{clamped:.4}"));
            out.push_str(",\"first\":");
            encode_node(first, out);
            out.push_str(",\"second\":");
            encode_node(second, out);
            out.push('}');
        }
    }
}

pub fn decode_json(input: &str) -> Result<SavedSession, String> {
    let mut parser = JsonParser::new(input);
    let val = parser.parse_value()?;
    parser.skip_ws();
    if !parser.is_eof() {
        return Err("trailing characters after JSON session".to_string());
    }
    parse_session(&val)
}

fn parse_session(val: &JsonVal) -> Result<SavedSession, String> {
    let obj = val.as_object()?;
    let version = obj.req_u64("version")? as u32;
    if version != VERSION {
        return Err(format!("unsupported session version {version}"));
    }
    let name = obj.req_string("name")?;
    validate_name(&name)?;
    let active_tab = obj.req_u64("active_tab")? as usize;
    let tabs_val = obj.req_array("tabs")?;
    let mut tabs = Vec::with_capacity(tabs_val.len());
    for item in tabs_val {
        tabs.push(parse_tab(item)?);
    }
    Ok(SavedSession {
        version,
        name,
        active_tab,
        tabs,
    })
}

fn parse_tab(val: &JsonVal) -> Result<SavedTab, String> {
    let obj = val.as_object()?;
    let title = obj.opt_string("title")?;
    let layout = obj.req_value("layout").and_then(parse_node)?;
    Ok(SavedTab { title, layout })
}

fn parse_node(val: &JsonVal) -> Result<SavedNode, String> {
    let obj = val.as_object()?;
    let kind = obj.req_string("kind")?;
    match kind.as_str() {
        "pane" => {
            let cwd = obj.opt_string("cwd")?;
            let active = obj.req_bool("active")?;
            Ok(SavedNode::Pane { cwd, active })
        }
        "split" => {
            let axis = match obj.req_string("axis")?.as_str() {
                "horizontal" => Axis::Horizontal,
                "vertical" => Axis::Vertical,
                other => return Err(format!("invalid split axis '{other}'")),
            };
            let ratio = obj.req_f32("ratio")?;
            if !ratio.is_finite() {
                return Err("split ratio is not finite".to_string());
            }
            let first = Box::new(obj.req_value("first").and_then(parse_node)?);
            let second = Box::new(obj.req_value("second").and_then(parse_node)?);
            Ok(SavedNode::Split {
                axis,
                ratio: ratio.clamp(MIN_RATIO, MAX_RATIO),
                first,
                second,
            })
        }
        other => Err(format!("unknown layout node kind '{other}'")),
    }
}

enum JsonVal {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonVal>),
    Object(Vec<(String, JsonVal)>),
}

impl JsonVal {
    fn as_object(&self) -> Result<JsonObj<'_>, String> {
        match self {
            JsonVal::Object(fields) => Ok(JsonObj(fields)),
            _ => Err("expected JSON object".to_string()),
        }
    }
}

struct JsonObj<'a>(&'a [(String, JsonVal)]);

impl<'a> JsonObj<'a> {
    fn req_value(&self, key: &str) -> Result<&'a JsonVal, String> {
        self.0
            .iter()
            .rfind(|(k, _)| k == key)
            .map(|(_, v)| v)
            .ok_or_else(|| format!("missing field '{key}'"))
    }

    fn req_string(&self, key: &str) -> Result<String, String> {
        match self.req_value(key)? {
            JsonVal::String(s) => Ok(s.clone()),
            _ => Err(format!("field '{key}' must be a string")),
        }
    }

    fn opt_string(&self, key: &str) -> Result<Option<String>, String> {
        match self.0.iter().rfind(|(k, _)| k == key).map(|(_, v)| v) {
            None | Some(JsonVal::Null) => Ok(None),
            Some(JsonVal::String(s)) => Ok(Some(s.clone())),
            _ => Err(format!("field '{key}' must be a string or null")),
        }
    }

    fn req_bool(&self, key: &str) -> Result<bool, String> {
        match self.req_value(key)? {
            JsonVal::Bool(b) => Ok(*b),
            _ => Err(format!("field '{key}' must be a boolean")),
        }
    }

    fn req_u64(&self, key: &str) -> Result<u64, String> {
        match self.req_value(key)? {
            JsonVal::Number(n) if *n >= 0.0 && n.is_finite() => Ok(*n as u64),
            _ => Err(format!("field '{key}' must be a non-negative number")),
        }
    }

    fn req_f32(&self, key: &str) -> Result<f32, String> {
        match self.req_value(key)? {
            JsonVal::Number(n) if n.is_finite() => Ok(*n as f32),
            _ => Err(format!("field '{key}' must be a number")),
        }
    }

    fn req_array(&self, key: &str) -> Result<&'a [JsonVal], String> {
        match self.req_value(key)? {
            JsonVal::Array(items) => Ok(items),
            _ => Err(format!("field '{key}' must be an array")),
        }
    }
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            bytes: input.as_bytes(),
            pos: 0,
        }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, expected: u8) -> Result<(), String> {
        self.skip_ws();
        match self.next() {
            Some(b) if b == expected => Ok(()),
            _ => Err(format!(
                "expected '{}' at byte {}",
                expected as char, self.pos
            )),
        }
    }

    fn parse_value(&mut self) -> Result<JsonVal, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => self.parse_string().map(JsonVal::String),
            Some(b't') => self.parse_lit(b"true").map(|()| JsonVal::Bool(true)),
            Some(b'f') => self.parse_lit(b"false").map(|()| JsonVal::Bool(false)),
            Some(b'n') => self.parse_lit(b"null").map(|()| JsonVal::Null),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(other) => Err(format!(
                "unexpected byte '{}' at {}",
                other as char, self.pos
            )),
            None => Err("unexpected end of JSON".to_string()),
        }
    }

    fn parse_lit(&mut self, lit: &[u8]) -> Result<(), String> {
        if self.bytes.get(self.pos..self.pos + lit.len()) == Some(lit) {
            self.pos += lit.len();
            Ok(())
        } else {
            Err(format!("invalid JSON literal at byte {}", self.pos))
        }
    }

    fn parse_number(&mut self) -> Result<JsonVal, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let slice = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| "invalid UTF-8 in number".to_string())?;
        let n = slice
            .parse::<f64>()
            .map_err(|_| format!("invalid JSON number '{slice}'"))?;
        Ok(JsonVal::Number(n))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        while let Some(b) = self.next() {
            match b {
                b'"' => return Ok(out),
                b'\\' => match self.next() {
                    Some(b'"') => out.push('"'),
                    Some(b'\\') => out.push('\\'),
                    Some(b'/') => out.push('/'),
                    Some(b'b') => out.push('\x08'),
                    Some(b'f') => out.push('\x0c'),
                    Some(b'n') => out.push('\n'),
                    Some(b'r') => out.push('\r'),
                    Some(b't') => out.push('\t'),
                    Some(b'u') => {
                        let code = self.parse_hex4()?;
                        let ch = char::from_u32(u32::from(code))
                            .ok_or_else(|| "invalid unicode escape".to_string())?;
                        out.push(ch);
                    }
                    _ => return Err("invalid JSON escape".to_string()),
                },
                byte if byte < 0x20 => {
                    return Err("unescaped control character in JSON".to_string());
                }
                first => {
                    let rest = &self.bytes[self.pos - 1..];
                    let s = std::str::from_utf8(rest)
                        .map_err(|_| "invalid UTF-8 in JSON string".to_string())?;
                    let ch = s
                        .chars()
                        .next()
                        .ok_or_else(|| "truncated string".to_string())?;
                    if first >= 0x80 {
                        self.pos += ch.len_utf8() - 1;
                    }
                    out.push(ch);
                }
            }
        }
        Err("unclosed JSON string".to_string())
    }

    fn parse_hex4(&mut self) -> Result<u16, String> {
        let mut val = 0u16;
        for _ in 0..4 {
            let b = self
                .next()
                .ok_or_else(|| "truncated \\u escape".to_string())?;
            let digit = (b as char)
                .to_digit(16)
                .ok_or_else(|| "invalid hex digit in \\u escape".to_string())?;
            val = (val << 4) | (digit as u16);
        }
        Ok(val)
    }

    fn parse_array(&mut self) -> Result<JsonVal, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonVal::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.next() {
                Some(b',') => continue,
                Some(b']') => return Ok(JsonVal::Array(items)),
                _ => return Err(format!("expected ',' or ']' at byte {}", self.pos)),
            }
        }
    }

    fn parse_object(&mut self) -> Result<JsonVal, String> {
        self.expect(b'{')?;
        let mut fields = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonVal::Object(fields));
        }
        loop {
            let key = self.parse_string()?;
            self.expect(b':')?;
            let val = self.parse_value()?;
            fields.push((key, val));
            self.skip_ws();
            match self.next() {
                Some(b',') => continue,
                Some(b'}') => return Ok(JsonVal::Object(fields)),
                _ => return Err(format!("expected ',' or '}}' at byte {}", self.pos)),
            }
        }
    }
}
