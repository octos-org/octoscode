//! What the review commands share: JSON that keeps its keys in order,
//! the review's state file and its lock, and how review reports are read.
//!
//! `review-state.json` is written by one command at a time: every command
//! that changes it holds an exclusive lock on `.review-state.lock` from
//! reading the state to replacing the file, and the file is replaced
//! atomically. Readers hold the same lock shared and create nothing.
//!
//! The state file, the receipts and the digests in them are also written
//! and read by `scripts/olp-review-evidence.py`, so values are compared
//! and serialised the way that script does it: an object's keys stay in
//! the order they were written, `1` equals `1.0`, and the digest of a
//! ledger turn is taken over sorted keys, compact, ASCII-escaped.

use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use fs2::FileExt;
use serde_json::Number;
use sha2::{Digest, Sha256};

pub const PROTOCOL: &str = "olp-review-evidence/v1";
pub const STATE_FILENAME: &str = "review-state.json";
const LOCK_FILENAME: &str = ".review-state.lock";
pub const LANES: [&str; 2] = ["glm", "k3"];

/// A refusal: printed as `{"error": {"code", "message"}}`, exit 1.
#[derive(Debug)]
pub struct Fail {
    pub code: String,
    pub message: String,
}

pub type Res<T> = Result<T, Fail>;

pub fn fail(code: &str, message: impl Into<String>) -> Fail {
    Fail {
        code: code.to_string(),
        message: message.into(),
    }
}

/// Something outside the review's rules went wrong (a file that cannot be
/// read or written).
pub fn broken(what: impl std::fmt::Display) -> Fail {
    fail("internal-error", what.to_string())
}

// ---------------------------------------------------------------------------
// JSON with keys in the order written
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum J {
    Null,
    Bool(bool),
    Num(Number),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

static NULL: J = J::Null;

impl<'de> serde::Deserialize<'de> for J {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<J, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = J;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a JSON value")
            }
            fn visit_unit<E>(self) -> Result<J, E> {
                Ok(J::Null)
            }
            fn visit_bool<E>(self, v: bool) -> Result<J, E> {
                Ok(J::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<J, E> {
                Ok(J::Num(v.into()))
            }
            fn visit_u64<E>(self, v: u64) -> Result<J, E> {
                Ok(J::Num(v.into()))
            }
            fn visit_f64<E>(self, v: f64) -> Result<J, E> {
                Ok(Number::from_f64(v).map_or(J::Null, J::Num))
            }
            fn visit_str<E>(self, v: &str) -> Result<J, E> {
                Ok(J::Str(v.to_string()))
            }
            fn visit_string<E>(self, v: String) -> Result<J, E> {
                Ok(J::Str(v))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<J, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(J::Arr(items))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<J, A::Error> {
                let mut out = J::Obj(Vec::new());
                while let Some((key, value)) = map.next_entry::<String, J>()? {
                    // A key written twice keeps its first place and its
                    // last value.
                    out.set(&key, value);
                }
                Ok(out)
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

impl From<&str> for J {
    fn from(value: &str) -> J {
        J::Str(value.to_string())
    }
}
impl From<String> for J {
    fn from(value: String) -> J {
        J::Str(value)
    }
}
impl From<bool> for J {
    fn from(value: bool) -> J {
        J::Bool(value)
    }
}
impl From<i64> for J {
    fn from(value: i64) -> J {
        J::Num(value.into())
    }
}
impl From<u64> for J {
    fn from(value: u64) -> J {
        J::Num(value.into())
    }
}
impl From<Option<&str>> for J {
    fn from(value: Option<&str>) -> J {
        value.map_or(J::Null, J::from)
    }
}
impl From<Vec<J>> for J {
    fn from(value: Vec<J>) -> J {
        J::Arr(value)
    }
}

/// An object from `(key, value)` pairs.
pub fn obj(pairs: Vec<(&str, J)>) -> J {
    J::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

pub fn strings(items: &[String]) -> J {
    J::Arr(items.iter().cloned().map(J::Str).collect())
}

impl J {
    pub fn parse(text: &str) -> Result<J, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// The value under `key`; null when this is not an object or has no
    /// such key.
    pub fn at(&self, key: &str) -> &J {
        self.get(key).unwrap_or(&NULL)
    }

    pub fn get(&self, key: &str) -> Option<&J> {
        match self {
            J::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut J> {
        match self {
            J::Obj(pairs) => pairs.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Put `value` under `key`: in place when the key is there, else last.
    pub fn set(&mut self, key: &str, value: J) {
        if let J::Obj(pairs) = self {
            match pairs.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = value,
                None => pairs.push((key.to_string(), value)),
            }
        }
    }

    /// The object under `key`, made (in place of anything else) when it
    /// is not one.
    pub fn child(&mut self, key: &str) -> &mut J {
        if !matches!(self.get(key), Some(J::Obj(_))) {
            self.set(key, J::Obj(Vec::new()));
        }
        self.get_mut(key).expect("just set")
    }

    pub fn is_null(&self) -> bool {
        matches!(self, J::Null)
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            J::Str(s) => Some(s),
            _ => None,
        }
    }

    /// A string with something in it.
    pub fn text(&self) -> Option<&str> {
        self.str().filter(|s| !s.is_empty())
    }

    pub fn items(&self) -> Option<&Vec<J>> {
        match self {
            J::Arr(items) => Some(items),
            _ => None,
        }
    }

    pub fn pairs(&self) -> Option<&Vec<(String, J)>> {
        match self {
            J::Obj(pairs) => Some(pairs),
            _ => None,
        }
    }

    pub fn is_obj(&self) -> bool {
        matches!(self, J::Obj(_))
    }

    /// A whole number (not a boolean, not `1.0`).
    pub fn int(&self) -> Option<i64> {
        match self {
            J::Num(n) if n.is_i64() => n.as_i64(),
            J::Num(n) if n.is_u64() => Some(i64::MAX),
            _ => None,
        }
    }

    pub fn is_true(&self) -> bool {
        matches!(self, J::Bool(true))
    }

    /// What `if value:` means in the script.
    pub fn truthy(&self) -> bool {
        match self {
            J::Null => false,
            J::Bool(b) => *b,
            J::Num(n) => n.as_f64().is_some_and(|f| f != 0.0),
            J::Str(s) => !s.is_empty(),
            J::Arr(items) => !items.is_empty(),
            J::Obj(pairs) => !pairs.is_empty(),
        }
    }

    fn number(&self) -> Option<f64> {
        match self {
            J::Bool(b) => Some(f64::from(u8::from(*b))),
            J::Num(n) => n.as_f64(),
            _ => None,
        }
    }

    /// Equal as the script's `==` has it: `1 == 1.0 == True`, and two
    /// objects are equal whatever the order of their keys.
    pub fn same(&self, other: &J) -> bool {
        match (self, other) {
            (J::Null, J::Null) => true,
            (J::Str(a), J::Str(b)) => a == b,
            (J::Arr(a), J::Arr(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.same(y)),
            (J::Obj(a), J::Obj(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, v)| other.get(k).is_some_and(|w| v.same(w)))
            }
            (J::Num(a), J::Num(b)) if !a.is_f64() && !b.is_f64() => a == b,
            _ => match (self.number(), other.number()) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            },
        }
    }

    /// Is the string `text`.
    pub fn is(&self, text: &str) -> bool {
        self.str() == Some(text)
    }

    fn write_string(text: &str, ascii: bool, out: &mut String) {
        out.push('"');
        for c in text.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\x08' => out.push_str("\\b"),
                '\x0c' => out.push_str("\\f"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c if ascii && !c.is_ascii() => {
                    let mut units = [0u16; 2];
                    for unit in c.encode_utf16(&mut units) {
                        out.push_str(&format!("\\u{unit:04x}"));
                    }
                }
                c => out.push(c),
            }
        }
        out.push('"');
    }

    fn write(
        &self,
        ascii: bool,
        indent: Option<usize>,
        sorted: bool,
        depth: usize,
        out: &mut String,
    ) {
        let pad = |out: &mut String, depth: usize| {
            if let Some(width) = indent {
                out.push('\n');
                out.push_str(&" ".repeat(width * depth));
            }
        };
        match self {
            J::Null => out.push_str("null"),
            J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            J::Num(n) => out.push_str(&number_text(n)),
            J::Str(s) => J::write_string(s, ascii, out),
            J::Arr(items) if items.is_empty() => out.push_str("[]"),
            J::Obj(pairs) if pairs.is_empty() => out.push_str("{}"),
            J::Arr(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    pad(out, depth + 1);
                    item.write(ascii, indent, sorted, depth + 1, out);
                }
                pad(out, depth);
                out.push(']');
            }
            J::Obj(pairs) => {
                let mut order: Vec<&(String, J)> = pairs.iter().collect();
                if sorted {
                    order.sort_by(|a, b| a.0.cmp(&b.0));
                }
                out.push('{');
                for (i, (key, value)) in order.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    pad(out, depth + 1);
                    J::write_string(key, ascii, out);
                    out.push(':');
                    if indent.is_some() {
                        out.push(' ');
                    }
                    value.write(ascii, indent, sorted, depth + 1, out);
                }
                pad(out, depth);
                out.push('}');
            }
        }
    }

    /// Indented by one space, non-ASCII kept: how the state file and the
    /// commands' answers are written.
    pub fn pretty(&self) -> String {
        let mut out = String::new();
        self.write(false, Some(1), false, 0, &mut out);
        out
    }

    /// Keys sorted, no spaces, non-ASCII escaped: the text digests are
    /// taken over.
    pub fn canonical(&self) -> String {
        let mut out = String::new();
        self.write(true, None, true, 0, &mut out);
        out
    }

    /// As the script's messages show a value with `!r`: `None`, `'text'`.
    pub fn repr(&self) -> String {
        match self {
            J::Null => "None".to_string(),
            J::Bool(true) => "True".to_string(),
            J::Bool(false) => "False".to_string(),
            J::Num(n) => number_text(n),
            J::Str(s) => repr_str(s),
            J::Arr(items) => {
                let inner: Vec<String> = items.iter().map(J::repr).collect();
                format!("[{}]", inner.join(", "))
            }
            J::Obj(pairs) => {
                let inner: Vec<String> = pairs
                    .iter()
                    .map(|(k, v)| format!("{}: {}", repr_str(k), v.repr()))
                    .collect();
                format!("{{{}}}", inner.join(", "))
            }
        }
    }

    /// As the script's messages show a value: a string as it is, anything
    /// else as [`J::repr`].
    pub fn shown(&self) -> String {
        match self {
            J::Str(s) => s.clone(),
            other => other.repr(),
        }
    }
}

/// A number as the script writes it: `1`, `1.0`, `1e-05`, `1e+16`.
fn number_text(n: &Number) -> String {
    if !n.is_f64() {
        return n.to_string();
    }
    let value = n.as_f64().unwrap_or(0.0);
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0"
        } else {
            "0.0"
        }
        .to_string();
    }
    // Shortest digits that read back as the same number, then the
    // script's layout: plain between 1e-4 and 1e16, else an exponent of at
    // least two digits with its sign.
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let sign = if negative { "-" } else { "" };
    if (-4..16).contains(&exponent) {
        let body = if exponent < 0 {
            format!("0.{}{digits}", "0".repeat((-exponent - 1) as usize))
        } else {
            let whole = exponent as usize + 1;
            if digits.len() <= whole {
                format!("{digits}{}.0", "0".repeat(whole - digits.len()))
            } else {
                format!("{}.{}", &digits[..whole], &digits[whole..])
            }
        };
        return format!("{sign}{body}");
    }
    let (first, rest) = digits.split_at(1);
    let point = if rest.is_empty() {
        String::new()
    } else {
        format!(".{rest}")
    };
    let exp_sign = if exponent < 0 { '-' } else { '+' };
    format!("{sign}{first}{point}e{exp_sign}{:02}", exponent.abs())
}

pub fn repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// An optional string in a message: the text, or `None`.
pub fn or_none(value: Option<&str>) -> String {
    value.unwrap_or("None").to_string()
}

/// An optional string in a message with `!r`: `'text'`, or `None`.
pub fn repr_opt(value: Option<&str>) -> String {
    value.map_or_else(|| "None".to_string(), repr_str)
}

// ---------------------------------------------------------------------------
// text and paths
// ---------------------------------------------------------------------------

pub fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

pub fn strip(text: &str) -> &str {
    text.trim_matches(is_space)
}

pub fn words(text: &str) -> Vec<&str> {
    text.split(is_space).filter(|w| !w.is_empty()).collect()
}

/// Lines without their endings; every line break Unicode knows ends one.
pub fn lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        let ends = matches!(
            c,
            '\n' | '\r'
                | '\x0b'
                | '\x0c'
                | '\x1c'
                | '\x1d'
                | '\x1e'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if !ends {
            continue;
        }
        out.push(&text[start..at]);
        start = at + c.len_utf8();
        if c == '\r' && matches!(chars.peek(), Some((_, '\n'))) {
            chars.next();
            start += 1;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

fn unix_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// A file's text: invalid UTF-8 replaced, line endings made `\n`.
pub fn read_lossy(path: &Path) -> std::io::Result<String> {
    Ok(unix_newlines(&String::from_utf8_lossy(&std::fs::read(
        path,
    )?)))
}

/// A file's text; an error when it is not UTF-8.
pub fn read_strict(path: &Path) -> std::io::Result<String> {
    let text = String::from_utf8(std::fs::read(path)?)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(unix_newlines(&text))
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    Ok(sha256_bytes(&std::fs::read(path)?))
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// A path argument without `.` parts, doubled or trailing slashes.
pub fn path_text(arg: &str) -> String {
    let lead = if arg.starts_with("//") && !arg.starts_with("///") {
        "//"
    } else if arg.starts_with('/') {
        "/"
    } else {
        ""
    };
    let parts: Vec<&str> = arg
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    let joined = format!("{lead}{}", parts.join("/"));
    if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

pub fn path_of(arg: &str) -> PathBuf {
    PathBuf::from(path_text(arg))
}

/// The absolute path with symlinks followed as far as the path exists;
/// what does not exist is kept as written, `..` taken literally.
pub fn resolve(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut done = PathBuf::new();
    let mut real = true;
    for part in absolute.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                done.pop();
            }
            other => {
                done.push(other);
                if real {
                    match done.canonicalize() {
                        Ok(found) => done = found,
                        Err(_) => real = false,
                    }
                }
            }
        }
    }
    done
}

pub fn show(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

// ---------------------------------------------------------------------------
// the state file and its lock
// ---------------------------------------------------------------------------

/// Replace `path` with `value`: a temporary file beside it, flushed to
/// disk, then renamed over it.
pub fn atomic_write_json(path: &Path, value: &J) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".tmp-state-")
        .tempfile_in(dir)?;
    tmp.write_all(format!("{}\n", value.pretty()).as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// The state, or an empty object when there is none yet.
pub fn load_state(review_dir: &Path) -> Res<J> {
    let path = review_dir.join(STATE_FILENAME);
    if !path.exists() {
        return Ok(J::Obj(Vec::new()));
    }
    let text = read_strict(&path).map_err(|e| broken(format!("{}: {e}", path.display())))?;
    J::parse(&text).map_err(|e| {
        fail(
            "state-corrupt",
            format!("{} 不是合法 JSON: {e}", path.display()),
        )
    })
}

pub fn save_state(review_dir: &Path, state: &mut J) -> Res<()> {
    state.set("updated_unix", J::from(now_unix()));
    atomic_write_json(&review_dir.join(STATE_FILENAME), state)
        .map_err(|e| broken(format!("写 {STATE_FILENAME} 失败: {e}")))
}

/// The review's lock, held until dropped.
pub struct ReviewLock {
    _file: std::fs::File,
}

/// Lock the review: exclusive for a command that writes (the directory
/// and the lock file are made), shared for one that only reads (nothing
/// is made; no lock file means the review cannot be read).
pub fn review_lock(review_dir: &Path, read_only: bool) -> Res<ReviewLock> {
    let lock = review_dir.join(LOCK_FILENAME);
    if read_only {
        let unavailable = |what: &str, e: std::io::Error| {
            fail(
                "review-lock-unavailable",
                format!("无法{what}评审锁: {}: {e}", review_dir.display()),
            )
        };
        let file = std::fs::File::open(&lock).map_err(|e| unavailable("读取", e))?;
        FileExt::lock_shared(&file).map_err(|e| unavailable("获取", e))?;
        return Ok(ReviewLock { _file: file });
    }
    std::fs::create_dir_all(review_dir).map_err(broken)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock)
        .map_err(broken)?;
    FileExt::lock_exclusive(&file).map_err(broken)?;
    Ok(ReviewLock { _file: file })
}

// ---------------------------------------------------------------------------
// review reports
// ---------------------------------------------------------------------------

static FRONTMATTER: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    fancy_regex::Regex::new(r"(?s)\A---\s*\n(.*?)\n---\s*\n").expect("a pattern written here")
});
static JSON_BLOCK: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    fancy_regex::Regex::new(r"(?s)```json\s*\n(.*?)```").expect("a pattern written here")
});

/// The `key: value` lines between a report's leading `---` lines.
pub struct Frontmatter {
    /// The report has no frontmatter.
    pub missing: bool,
    fields: Vec<(String, String)>,
    /// Of the whole file.
    pub sha256: String,
    /// A key appears twice.
    pub repeated: bool,
}

impl Frontmatter {
    pub fn of_text(text: &str, sha256: String) -> Frontmatter {
        let mut out = Frontmatter {
            missing: true,
            fields: Vec::new(),
            sha256,
            repeated: false,
        };
        let Ok(Some(found)) = FRONTMATTER.captures(text) else {
            return out;
        };
        out.missing = false;
        for line in lines(found.get(1).map_or("", |m| m.as_str())) {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let (key, value) = (strip(key).to_string(), strip(value).to_string());
            match out.fields.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => {
                    slot.1 = value;
                    out.repeated = true;
                }
                None => out.fields.push((key, value)),
            }
        }
        out
    }

    pub fn read(path: &Path) -> Res<Frontmatter> {
        let bytes = std::fs::read(path).map_err(|e| broken(format!("{}: {e}", path.display())))?;
        let text = unix_newlines(&String::from_utf8_lossy(&bytes));
        Ok(Frontmatter::of_text(&text, sha256_bytes(&bytes)))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// A value with something in it.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.get(key).filter(|v| !v.is_empty())
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The report's first ```` ```json ```` block when it is an object.
fn json_block(path: &Path) -> Res<Option<J>> {
    let text = read_lossy(path).map_err(|e| broken(format!("{}: {e}", path.display())))?;
    let Ok(Some(found)) = JSON_BLOCK.captures(&text) else {
        return Ok(None);
    };
    let block = J::parse(found.get(1).map_or("", |m| m.as_str())).map_err(|e| {
        fail(
            "claims-block-invalid",
            format!("{} claims 块 JSON 非法: {e}", file_name(path)),
        )
    })?;
    Ok(Some(block).filter(J::is_obj))
}

/// The claims of a first review: `{"claims": [{"id", "verdict",
/// "evidence"}]}`. No block at all is an empty list.
pub fn parse_claims(path: &Path) -> Res<Vec<J>> {
    let name = file_name(path);
    let Some(block) = json_block(path)? else {
        return Ok(Vec::new());
    };
    let claims = block.at("claims").items().filter(|c| !c.is_empty());
    let Some(claims) = claims else {
        return Err(fail(
            "claims-block-invalid",
            format!("{name} claims 块缺非空 claims 数组"),
        ));
    };
    for claim in claims {
        let Some(id) = claim.at("id").text().filter(|_| claim.is_obj()) else {
            return Err(fail(
                "claims-block-invalid",
                format!("{name} claim 缺合法 id"),
            ));
        };
        // A first review says approve, request-changes or comment; the
        // words of a cross report do not belong here.
        let verdict = claim.at("verdict");
        if !["approve", "request-changes", "comment"].contains(&verdict.str().unwrap_or("")) {
            return Err(fail(
                "claims-verdict-invalid",
                format!(
                    "{name} claim[{id}] verdict={} 非法(合法: ['approve', 'comment', 'request-changes'])",
                    verdict.repr()
                ),
            ));
        }
    }
    Ok(claims.clone())
}

/// The claims of a cross report: `{"cross_claims": [{"id", "verdict",
/// "reference"?}]}`. None when the report has no such block; a claim is
/// covered only by an entry here, never by its id appearing in the text.
pub fn parse_cross_claims(path: &Path) -> Res<Option<Vec<J>>> {
    let name = file_name(path);
    let Some(block) = json_block(path)? else {
        return Ok(None);
    };
    let claims = block.at("cross_claims");
    if claims.is_null() {
        return Ok(None);
    }
    let Some(claims) = claims.items().filter(|c| !c.is_empty()) else {
        return Err(fail(
            "cross-claim-invalid",
            format!("{name} cross_claims 缺非空数组"),
        ));
    };
    for claim in claims {
        let Some(id) = claim.at("id").text().filter(|_| claim.is_obj()) else {
            return Err(fail(
                "cross-claim-invalid",
                format!("{name} cross claim 缺合法 id"),
            ));
        };
        let verdict = claim.at("verdict");
        if !["accept", "refute", "pending"].contains(&verdict.str().unwrap_or("")) {
            return Err(fail(
                "cross-claim-invalid",
                format!(
                    "{name} claim {id} verdict={} 非法(须为 ('accept', 'refute', 'pending'))",
                    verdict.shown()
                ),
            ));
        }
        let reference = claim.at("reference");
        if !reference.is_null() && !reference.is_obj() {
            return Err(fail(
                "cross-claim-invalid",
                format!("{name} claim {id} reference 须为对象"),
            ));
        }
    }
    Ok(Some(claims.clone()))
}

/// The repository's real HEAD. Never what the caller says it is.
pub fn resolve_head(repo: &Path) -> Res<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output();
    if let Ok(out) = out {
        let head = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if out.status.success() && is_sha(&head) {
            return Ok(head);
        }
    }
    Err(fail(
        "head-unresolvable",
        format!("无法解析 repo 真实 HEAD: {}", repo.display()),
    ))
}

/// Forty lowercase hex digits.
pub fn is_sha(text: &str) -> bool {
    text.len() == 40
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A turn number as a report or the state gives it: a whole number above
/// zero, as a number or as digits.
pub fn positive_turn(value: &J) -> Option<u64> {
    let digits = match value {
        J::Str(s) => s.clone(),
        J::Num(n) if !n.is_f64() => n.to_string(),
        _ => return None,
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|n| *n > 0)
}

/// Digits only, as a number.
pub fn digits(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_keeps_key_order_and_writes_like_the_script() {
        let value = J::parse(r#"{"b": 1, "a": [true, null, 1.5, "é\n"], "c": {}}"#).unwrap();
        assert_eq!(
            value.pretty(),
            "{\n \"b\": 1,\n \"a\": [\n  true,\n  null,\n  1.5,\n  \"é\\n\"\n ],\n \"c\": {}\n}"
        );
        assert_eq!(
            value.canonical(),
            "{\"a\":[true,null,1.5,\"\\u00e9\\n\"],\"b\":1,\"c\":{}}"
        );
    }

    #[test]
    fn numbers_are_written_like_the_script() {
        let text = |s: &str| J::parse(s).unwrap().canonical();
        assert_eq!(text("1.0"), "1.0");
        assert_eq!(text("0.00001"), "1e-05");
        assert_eq!(text("0.0001"), "0.0001");
        assert_eq!(text("1e16"), "1e+16");
        assert_eq!(text("123456789012345.6"), "123456789012345.6");
        assert_eq!(text("1.5e-7"), "1.5e-07");
        assert_eq!(text("-2.5"), "-2.5");
        assert_eq!(text("1e22"), "1e+22");
        assert_eq!(text("100.0"), "100.0");
    }

    #[test]
    fn equality_follows_the_script() {
        let a = J::parse(r#"{"x": 1, "y": [1.0]}"#).unwrap();
        let b = J::parse(r#"{"y": [1], "x": 1.0}"#).unwrap();
        assert!(a.same(&b));
        assert!(!J::parse("\"1\"").unwrap().same(&J::parse("1").unwrap()));
    }

    #[test]
    fn a_repeated_key_keeps_its_place_and_last_value() {
        let value = J::parse(r#"{"a": 1, "b": 2, "a": 3}"#).unwrap();
        assert_eq!(value.canonical(), r#"{"a":3,"b":2}"#);
    }
}
