//! Tiny order-preserving JSON parser/printer for the Text tab's "JSON pretty" / "JSON minify" tools.
//!
//! `JSON.stringify(JSON.parse(s), null, 2)` keeps key order (integer-like keys first, ascending, like a JS object),
//! lets a duplicate key overwrite the earlier value in place, prints numbers the JS way (`1.0` -> `1`, `1e3` -> `1000`)
//! and escapes strings like `JSON.stringify`. serde_json's default map sorts keys, so the Text tab cannot use it.
//! Known gap: a lone `\ud800` surrogate escape is rejected (a Rust `String` cannot hold it).

#[derive(Clone, Debug, PartialEq)]
pub enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

struct P<'a> {
    b: &'a [u8],
    s: &'a str,
    i: usize,
}

pub fn parse(s: &str) -> Result<J, String> {
    let mut p = P { b: s.as_bytes(), s, i: 0 };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.b.len() {
        return Err("trailing data".into());
    }
    Ok(v)
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    fn eat(&mut self, lit: &str) -> bool {
        if self.s[self.i..].starts_with(lit) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }
    fn value(&mut self, depth: usize) -> Result<J, String> {
        if depth > 512 {
            return Err("too deep".into());
        }
        match self.b.get(self.i) {
            Some(b'{') => {
                self.i += 1;
                let mut kv: Vec<(String, J)> = Vec::new();
                self.ws();
                if self.eat("}") {
                    return Ok(J::Obj(kv));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.ws();
                    if !self.eat(":") {
                        return Err("expected :".into());
                    }
                    self.ws();
                    let v = self.value(depth + 1)?;
                    if let Some(slot) = kv.iter_mut().find(|(x, _)| *x == k) {
                        slot.1 = v; // a duplicate key overwrites in place
                    } else {
                        kv.push((k, v));
                    }
                    self.ws();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("}") {
                        break;
                    }
                    return Err("expected , or }".into());
                }
                // JS objects list canonical array-index keys first, ascending
                let (mut idx, rest): (Vec<_>, Vec<_>) = kv.into_iter().partition(|(k, _)| is_index(k));
                idx.sort_by_key(|(k, _)| k.parse::<u64>().unwrap_or(0));
                idx.extend(rest);
                Ok(J::Obj(idx))
            }
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.eat("]") {
                    return Ok(J::Arr(a));
                }
                loop {
                    self.ws();
                    a.push(self.value(depth + 1)?);
                    self.ws();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("]") {
                        break;
                    }
                    return Err("expected , or ]".into());
                }
                Ok(J::Arr(a))
            }
            Some(b'"') => Ok(J::Str(self.string()?)),
            Some(b't') if self.eat("true") => Ok(J::Bool(true)),
            Some(b'f') if self.eat("false") => Ok(J::Bool(false)),
            Some(b'n') if self.eat("null") => Ok(J::Null),
            Some(b'-') | Some(b'0'..=b'9') => self.number(),
            _ => Err("unexpected token".into()),
        }
    }
    fn number(&mut self) -> Result<J, String> {
        let st = self.i;
        if self.b[self.i] == b'-' {
            self.i += 1;
        }
        match self.b.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => self.digits(),
            _ => return Err("bad number".into()),
        }
        if self.b.get(self.i) == Some(&b'.') {
            self.i += 1;
            let d = self.i;
            self.digits();
            if self.i == d {
                return Err("bad number".into());
            }
        }
        if matches!(self.b.get(self.i), Some(b'e') | Some(b'E')) {
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+') | Some(b'-')) {
                self.i += 1;
            }
            let d = self.i;
            self.digits();
            if self.i == d {
                return Err("bad number".into());
            }
        }
        self.s[st..self.i].parse::<f64>().map(J::Num).map_err(|_| "bad number".into())
    }
    fn digits(&mut self) {
        while matches!(self.b.get(self.i), Some(b'0'..=b'9')) {
            self.i += 1;
        }
    }
    fn hex4(&mut self) -> Result<u32, String> {
        let h = self.s.get(self.i..self.i + 4).ok_or("bad escape")?;
        if !h.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("bad escape".into());
        }
        self.i += 4;
        u32::from_str_radix(h, 16).map_err(|_| "bad escape".into())
    }
    fn string(&mut self) -> Result<String, String> {
        if self.b.get(self.i) != Some(&b'"') {
            return Err("expected string".into());
        }
        self.i += 1;
        let mut out = String::new();
        loop {
            let c = self.s[self.i..].chars().next().ok_or("unterminated string")?;
            self.i += c.len_utf8();
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let e = self.b.get(self.i).copied().ok_or("bad escape")?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            if (0xD800..0xDC00).contains(&hi) {
                                if !self.eat("\\u") {
                                    return Err("lone surrogate".into());
                                }
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err("lone surrogate".into());
                                }
                                out.push(char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)).ok_or("bad escape")?);
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return Err("lone surrogate".into());
                            } else {
                                out.push(char::from_u32(hi).ok_or("bad escape")?);
                            }
                        }
                        _ => return Err("bad escape".into()),
                    }
                }
                c if (c as u32) < 0x20 => return Err("control character in string".into()),
                c => out.push(c),
            }
        }
    }
}

fn is_index(k: &str) -> bool {
    !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()) && (k == "0" || !k.starts_with('0')) && k.parse::<u64>().map_or(false, |n| n < 4_294_967_295)
}

/// Number::toString of ECMAScript (shortest digits, no exponent between 1e-6 and 1e21); NaN/Infinity never reach here.
pub fn js_num(x: f64) -> String {
    if x == 0.0 {
        return "0".into();
    }
    if !x.is_finite() {
        return "null".into();
    }
    let sign = if x < 0.0 { "-" } else { "" };
    let e = format!("{:e}", x.abs()); // d.ddde-7
    let (mant, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exp.parse::<i32>().unwrap_or(0) + 1;
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let ex = n - 1;
        let es = format!("e{}{}", if ex < 0 { '-' } else { '+' }, ex.abs());
        if k == 1 { format!("{digits}{es}") } else { format!("{}.{}{es}", &digits[..1], &digits[1..]) }
    };
    format!("{sign}{body}")
}

/// `JSON.stringify` string escaping.
pub fn js_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\u{8}' => o.push_str("\\b"),
            '\u{c}' => o.push_str("\\f"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// `JSON.stringify(v, null, indent)`; indent 0 = compact.
pub fn stringify(v: &J, indent: usize) -> String {
    let mut o = String::new();
    write(v, indent, 0, &mut o);
    o
}
fn write(v: &J, ind: usize, lvl: usize, o: &mut String) {
    let nl = |o: &mut String, l: usize| {
        if ind > 0 {
            o.push('\n');
            o.push_str(&" ".repeat(ind * l));
        }
    };
    match v {
        J::Null => o.push_str("null"),
        J::Bool(b) => o.push_str(if *b { "true" } else { "false" }),
        J::Num(n) => o.push_str(&js_num(*n)),
        J::Str(s) => o.push_str(&js_str(s)),
        J::Arr(a) if a.is_empty() => o.push_str("[]"),
        J::Arr(a) => {
            o.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    o.push(',');
                }
                nl(o, lvl + 1);
                write(x, ind, lvl + 1, o);
            }
            nl(o, lvl);
            o.push(']');
        }
        J::Obj(m) if m.is_empty() => o.push_str("{}"),
        J::Obj(m) => {
            o.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    o.push(',');
                }
                nl(o, lvl + 1);
                o.push_str(&js_str(k));
                o.push(':');
                if ind > 0 {
                    o.push(' ');
                }
                write(x, ind, lvl + 1, o);
            }
            nl(o, lvl);
            o.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pretty(s: &str) -> Result<String, String> {
        parse(s).map(|v| stringify(&v, 2))
    }
    #[test]
    fn pretty_matches_js() {
        assert_eq!(pretty(r#"{"a":[1,2],"b":"x"}"#).unwrap(), "{\n  \"a\": [\n    1,\n    2\n  ],\n  \"b\": \"x\"\n}");
        assert_eq!(pretty("[]").unwrap(), "[]");
        assert_eq!(pretty(" {\"a\":{},\"b\":[ ]} ").unwrap(), "{\n  \"a\": {},\n  \"b\": []\n}");
    }
    #[test]
    fn key_order_and_duplicates() {
        assert_eq!(stringify(&parse(r#"{"b":1,"2":2,"a":3,"1":4}"#).unwrap(), 0), r#"{"1":4,"2":2,"b":1,"a":3}"#);
        assert_eq!(stringify(&parse(r#"{"a":1,"b":2,"a":3}"#).unwrap(), 0), r#"{"a":3,"b":2}"#);
        // order of non-index keys is kept (serde_json's default map would sort them)
        assert_eq!(stringify(&parse(r#"{"z":1,"a":2}"#).unwrap(), 0), r#"{"z":1,"a":2}"#);
    }
    #[test]
    fn numbers_print_like_js() {
        let n = |s: &str| stringify(&parse(s).unwrap(), 0);
        assert_eq!(n("1.0"), "1");
        assert_eq!(n("1e3"), "1000");
        assert_eq!(n("-0"), "0");
        assert_eq!(n("0.1"), "0.1");
        assert_eq!(n("1e21"), "1e+21");
        assert_eq!(n("1.5e-7"), "1.5e-7");
        assert_eq!(n("123456789012345680000"), "123456789012345680000");
        assert_eq!(n("0.000001"), "0.000001");
        assert_eq!(n("1e999"), "null");
    }
    #[test]
    fn strings_escape_like_js() {
        let v = parse(r#""a\u0001\n\"\\/\u00e9 😀 \ud83d\ude00""#).unwrap();
        assert_eq!(stringify(&v, 0), "\"a\\u0001\\n\\\"\\\\/é 😀 😀\"");
    }
    #[test]
    fn rejects_what_js_rejects() {
        for bad in ["{nope", "", "[1,]", "{\"a\":1,}", "01", "1.", ".5", "'a'", "[1 2]", "\"a\nb\"", "{\"a\" 1}", "tru", "[1]x", "\"\\x\""] {
            assert!(parse(bad).is_err(), "{bad:?} should fail");
        }
    }
}
