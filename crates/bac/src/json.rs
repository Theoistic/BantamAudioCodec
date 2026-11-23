//! A small JSON reader and writer, for the label files people edit by hand.

use crate::error::{bad, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    /// in the order of the text
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn parse(s: &str) -> Result<Json> {
        let mut p = Parser { b: s.as_bytes(), pos: 0 };
        let v = p.value(0)?;
        p.space();
        if p.pos != p.b.len() {
            return Err(p.err("text after the value"));
        }
        Ok(v)
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(o) => o.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_obj(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Obj(o) => Some(o),
            _ => None,
        }
    }

    /// The text of the value. Objects inside arrays stay on one line, so that a list of events reads as a table.
    pub fn pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0, false);
        out.push('\n');
        out
    }

    fn write(&self, out: &mut String, indent: usize, flat: bool) {
        let pad = |out: &mut String, n: usize| {
            out.push('\n');
            out.extend(std::iter::repeat_n("  ", n));
        };
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(v) => out.push_str(if *v { "true" } else { "false" }),
            Json::Num(v) => {
                if v.fract() == 0.0 && v.abs() < 1e15 {
                    out.push_str(&format!("{}", *v as i64));
                } else {
                    out.push_str(&format!("{v}"));
                }
            }
            Json::Str(s) => write_str(out, s),
            Json::Arr(a) => {
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    if flat {
                        out.push(' ');
                    } else {
                        pad(out, indent + 1);
                    }
                    v.write(out, indent + 1, true);
                }
                if !a.is_empty() && !flat {
                    pad(out, indent);
                }
                out.push(']');
            }
            Json::Obj(o) => {
                out.push('{');
                for (i, (k, v)) in o.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    if flat {
                        if i > 0 {
                            out.push(' ');
                        }
                    } else {
                        pad(out, indent + 1);
                    }
                    write_str(out, k);
                    out.push_str(": ");
                    v.write(out, indent + 1, flat);
                }
                if !o.is_empty() && !flat {
                    pad(out, indent);
                }
                out.push('}');
            }
        }
    }
}

fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    b: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn err(&self, m: &str) -> crate::Error {
        bad(format!("JSON, byte {}: {m}", self.pos))
    }

    fn space(&mut self) {
        while matches!(self.b.get(self.pos), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.space();
        if self.b.get(self.pos) == Some(&c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json> {
        if depth > 64 {
            return Err(self.err("nested too deep"));
        }
        self.space();
        match self.b.get(self.pos).copied() {
            Some(b'{') => {
                self.pos += 1;
                let mut o = Vec::new();
                if !self.eat(b'}') {
                    loop {
                        self.space();
                        let k = self.string()?;
                        if !self.eat(b':') {
                            return Err(self.err("expected ':'"));
                        }
                        o.push((k, self.value(depth + 1)?));
                        if self.eat(b'}') {
                            break;
                        }
                        if !self.eat(b',') {
                            return Err(self.err("expected ',' or '}'"));
                        }
                    }
                }
                Ok(Json::Obj(o))
            }
            Some(b'[') => {
                self.pos += 1;
                let mut a = Vec::new();
                if !self.eat(b']') {
                    loop {
                        a.push(self.value(depth + 1)?);
                        if self.eat(b']') {
                            break;
                        }
                        if !self.eat(b',') {
                            return Err(self.err("expected ',' or ']'"));
                        }
                    }
                }
                Ok(Json::Arr(a))
            }
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.word("true", Json::Bool(true)),
            Some(b'f') => self.word("false", Json::Bool(false)),
            Some(b'n') => self.word("null", Json::Null),
            Some(_) => {
                let from = self.pos;
                while matches!(self.b.get(self.pos), Some(b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')) {
                    self.pos += 1;
                }
                std::str::from_utf8(&self.b[from..self.pos]).ok().and_then(|t| t.parse().ok()).map(Json::Num).ok_or_else(|| self.err("expected a value"))
            }
            None => Err(self.err("the text ends early")),
        }
    }

    fn word(&mut self, w: &str, v: Json) -> Result<Json> {
        if self.b[self.pos..].starts_with(w.as_bytes()) {
            self.pos += w.len();
            Ok(v)
        } else {
            Err(self.err("expected a value"))
        }
    }

    fn hex4(&mut self) -> Result<u32> {
        let t = self.b.get(self.pos..self.pos + 4).and_then(|t| std::str::from_utf8(t).ok()).ok_or_else(|| self.err("bad \\u escape"))?;
        let v = u32::from_str_radix(t, 16).map_err(|_| self.err("bad \\u escape"))?;
        self.pos += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String> {
        if self.b.get(self.pos) != Some(&b'"') {
            return Err(self.err("expected a string"));
        }
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            match self.b.get(self.pos).copied() {
                None => return Err(self.err("a string without its end")),
                Some(b'"') => {
                    self.pos += 1;
                    break;
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let c = self.b.get(self.pos).copied().ok_or_else(|| self.err("a string without its end"))?;
                    self.pos += 1;
                    let ch = match c {
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'u' => {
                            let mut v = self.hex4()?;
                            // two escapes that make one character beyond U+FFFF
                            if (0xD800..0xDC00).contains(&v) && self.b[self.pos..].starts_with(b"\\u") {
                                self.pos += 2;
                                let lo = self.hex4()?;
                                v = 0x10000 + ((v - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            char::from_u32(v).unwrap_or('\u{fffd}')
                        }
                        c => c as char,
                    };
                    out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                }
                Some(c) => {
                    out.push(c);
                    self.pos += 1;
                }
            }
        }
        String::from_utf8(out).map_err(|_| self.err("text that is not UTF-8"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_comes_back() {
        let src = r#"{"a": [1, 2.5, -3e2, true, null], "s": "x\"y\né😀", "o": {}, "e": []}"#;
        let v = Json::parse(src).unwrap();
        assert_eq!(v.get("s").unwrap().as_str().unwrap(), "x\"y\né😀");
        assert_eq!(v.get("a").unwrap().as_arr().unwrap()[2], Json::Num(-300.0));
        assert_eq!(Json::parse(&v.pretty()).unwrap(), v);
        assert!(Json::parse("{\"a\": }").is_err());
        assert!(Json::parse("[1, 2").is_err());
    }
}
