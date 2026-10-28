// Command bar: calculator (recursive descent, no eval) and offline unit conversion. Split out of command.rs; no behaviour change.

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Id(String),
    Op(char),
}

fn lex(s: &str) -> Option<Vec<Tok>> {
    let c: Vec<char> = s.chars().collect();
    let (mut i, mut t) = (0, Vec::new());
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch.is_ascii_digit() || ((ch == '.' || ch == ',') && c.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let (st, mut sep) = (i, false);
            while i < c.len() && (c[i].is_ascii_digit() || (!sep && (c[i] == '.' || c[i] == ','))) {
                sep |= c[i] == '.' || c[i] == ',';
                i += 1;
            }
            t.push(Tok::Num(c[st..i].iter().map(|&x| if x == ',' { '.' } else { x }).collect::<String>().parse().ok()?));
        } else if ch.is_alphabetic() {
            let st = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            t.push(Tok::Id(c[st..i].iter().collect::<String>().to_lowercase()));
        } else {
            t.push(Tok::Op(match ch {
                '+' | '-' | '*' | '/' | '^' | '%' | '(' | ')' => ch,
                '×' => '*',
                '÷' => '/',
                _ => return None,
            }));
            i += 1;
        }
    }
    Some(t)
}

struct Parser {
    t: Vec<Tok>,
    i: usize,
    depth: u32,
}

impl Parser {
    fn op(&self) -> Option<char> {
        match self.t.get(self.i) {
            Some(Tok::Op(c)) => Some(*c),
            _ => None,
        }
    }
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        while let Some(o @ ('+' | '-')) = self.op() {
            self.i += 1;
            let r = self.term()?;
            v = if o == '+' { v + r } else { v - r };
        }
        Some(v)
    }
    fn term(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        while let Some(o @ ('*' | '/' | '%')) = self.op() {
            self.i += 1; // a '%' left here is binary: postfix() already took the percent-sign case
            let r = self.unary()?;
            v = match o {
                '*' => v * r,
                '/' => v / r,
                _ => v % r,
            };
        }
        Some(v)
    }
    fn unary(&mut self) -> Option<f64> {
        self.depth += 1;
        if self.depth > 64 {
            return None;
        }
        let r = match self.op() {
            Some('-') => {
                self.i += 1;
                self.unary().map(|v| -v)
            }
            Some('+') => {
                self.i += 1;
                self.unary()
            }
            _ => self.power(),
        };
        self.depth -= 1;
        r
    }
    fn power(&mut self) -> Option<f64> {
        let b = self.postfix()?;
        if self.op() == Some('^') {
            self.i += 1;
            return Some(b.powf(self.unary()?)); // right associative, allows 2^-1
        }
        Some(b)
    }
    fn postfix(&mut self) -> Option<f64> {
        let mut v = self.primary()?;
        // `50%` is a percent sign unless an operand follows (`10 % 3` is modulo).
        while self.op() == Some('%') && !matches!(self.t.get(self.i + 1), Some(Tok::Num(_) | Tok::Id(_) | Tok::Op('('))) {
            self.i += 1;
            v /= 100.0;
        }
        Some(v)
    }
    fn primary(&mut self) -> Option<f64> {
        let tok = self.t.get(self.i)?.clone();
        self.i += 1;
        match tok {
            Tok::Num(n) => Some(n),
            Tok::Op('(') => {
                self.depth += 1;
                let v = if self.depth > 64 { None } else { self.expr() };
                self.depth -= 1;
                (self.op() == Some(')')).then(|| self.i += 1).and(v)
            }
            Tok::Id(name) if self.op() == Some('(') => {
                self.i += 1;
                self.depth += 1;
                let v = if self.depth > 64 { None } else { self.expr() };
                self.depth -= 1;
                let v = (self.op() == Some(')')).then(|| self.i += 1).and(v)?;
                Some(match name.as_str() {
                    "sqrt" => v.sqrt(),
                    "sin" => v.sin(),
                    "cos" => v.cos(),
                    "tan" => v.tan(),
                    "log" => v.log10(),
                    "ln" => v.ln(),
                    "abs" => v.abs(),
                    "round" => v.round(),
                    "floor" => v.floor(),
                    "ceil" => v.ceil(),
                    _ => return None,
                })
            }
            Tok::Id(name) => match name.as_str() {
                "pi" => Some(std::f64::consts::PI),
                "e" => Some(std::f64::consts::E),
                _ => None,
            },
            _ => None,
        }
    }
}

/// Evaluate an arithmetic expression; None for anything malformed or non-finite.
pub(super) fn calc(s: &str) -> Option<f64> {
    if s.len() > 200 {
        return None;
    }
    let t = lex(s)?;
    if t.is_empty() {
        return None;
    }
    let mut p = Parser { t, i: 0, depth: 0 };
    let v = p.expr()?;
    (p.i == p.t.len() && v.is_finite()).then_some(v)
}

pub(super) fn fmt_num(v: f64) -> String {
    if v != 0.0 && !(1e-9..1e15).contains(&v.abs()) {
        return format!("{v:e}");
    }
    let s = format!("{v:.10}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() { "0".into() } else { s.to_string() }
}

/// (formatted result, expression as typed). A bare number is not a calculation.
pub(super) fn calc_row(q: &str) -> Option<(String, String)> {
    if matches!(lex(q)?.as_slice(), [Tok::Num(_)]) {
        return None;
    }
    calc(q).map(|v| (fmt_num(v), q.to_string()))
}

// ---- unit conversion (offline) ----

// (names, category, factor to the category base). Temperature is handled separately.
const UNITS: &[(&str, &str, f64)] = &[
    ("mm|millimeter|millimeters", "len", 0.001),
    ("cm|centimeter|centimeters", "len", 0.01),
    ("m|meter|meters|metre|metres", "len", 1.0),
    ("km|kilometer|kilometers|kilometre|kilometres", "len", 1000.0),
    ("in|inch|inches", "len", 0.0254),
    ("ft|foot|feet", "len", 0.3048),
    ("yd|yard|yards", "len", 0.9144),
    ("mi|mile|miles", "len", 1609.344),
    ("mg|milligram|milligrams", "mass", 1e-6),
    ("g|gram|grams", "mass", 1e-3),
    ("kg|kilogram|kilograms", "mass", 1.0),
    ("t|tonne|tonnes", "mass", 1000.0),
    ("oz|ounce|ounces", "mass", 0.028349523125),
    ("lb|lbs|pound|pounds", "mass", 0.45359237),
    ("bit|bits", "data", 0.125),
    ("byte|bytes", "data", 1.0),
    ("kb|kilobyte|kilobytes", "data", 1e3),
    ("mb|megabyte|megabytes", "data", 1e6),
    ("gb|gigabyte|gigabytes", "data", 1e9),
    ("tb|terabyte|terabytes", "data", 1e12),
    ("kib", "data", 1024.0),
    ("mib", "data", 1_048_576.0),
    ("gib", "data", 1_073_741_824.0),
    ("tib", "data", 1_099_511_627_776.0),
    ("ms|millisecond|milliseconds", "time", 0.001),
    ("s|sec|secs|second|seconds", "time", 1.0),
    ("min|mins|minute|minutes", "time", 60.0),
    ("h|hr|hrs|hour|hours", "time", 3600.0),
    ("d|day|days", "time", 86400.0),
    ("wk|week|weeks", "time", 604800.0),
    ("c|celsius", "temp", 1.0),
    ("f|fahrenheit", "temp", 1.0),
    ("k|kelvin", "temp", 1.0),
];

fn unit(name: &str) -> Option<(&'static str, f64, &'static str)> {
    let n = name.to_lowercase();
    UNITS.iter().find(|(names, _, _)| names.split('|').any(|x| x == n)).map(|(names, cat, f)| (*cat, *f, names.split('|').next().unwrap()))
}

/// "5 km in mi" -> (title, subtitle, copy text).
pub(super) fn convert(q: &str) -> Option<(String, String, String)> {
    let mut tok: Vec<String> = q.split_whitespace().map(str::to_string).collect();
    // glued "5km": split the first token at the first letter
    if let Some(first) = tok.first().cloned() {
        if let Some(p) = first.find(|c: char| c.is_alphabetic()) {
            if p > 0 {
                tok[0] = first[..p].to_string();
                tok.insert(1, first[p..].to_string());
            }
        }
    }
    let [num, from, sep, to] = tok.as_slice() else { return None };
    if !["in", "to", "as"].contains(&sep.to_lowercase().as_str()) {
        return None;
    }
    let v: f64 = num.replace(',', ".").parse().ok().filter(|v: &f64| v.is_finite())?;
    let ((c1, f1, _), (c2, f2, _)) = (unit(from)?, unit(to)?);
    if c1 != c2 {
        return None;
    }
    let r = if c1 == "temp" {
        let (a, b) = (from.to_lowercase(), to.to_lowercase());
        let cel = match a.chars().next()? {
            'f' => (v - 32.0) * 5.0 / 9.0,
            'k' => v - 273.15,
            _ => v,
        };
        match b.chars().next()? {
            'f' => cel * 9.0 / 5.0 + 32.0,
            'k' => cel + 273.15,
            _ => cel,
        }
    } else {
        v * f1 / f2
    };
    let text = format!("{} {}", fmt_num(r), to);
    Some((text.clone(), format!("{} {} in {}", fmt_num(v), from, to), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculator() {
        let ok: &[(&str, f64)] = &[
            ("1+2*3", 7.0),
            ("(1+2)*3", 9.0),
            ("2^3^2", 512.0),
            ("-2^2", -4.0),
            ("2^-1", 0.5),
            ("--3", 3.0),
            ("10-4-3", 3.0),
            ("8/4/2", 1.0),
            ("50%", 0.5),
            ("200*10%", 20.0),
            ("10 % 3", 1.0),
            ("2,5+1", 3.5),
            ("1.5*2", 3.0),
            ("sqrt(16)+abs(-2)", 6.0),
            ("round(2.6)+floor(2.6)+ceil(2.1)", 8.0),
            ("ln(e)", 1.0),
            ("log(1000)", 3.0),
            ("2*pi", 2.0 * std::f64::consts::PI),
            ("3 × 4 ÷ 2", 6.0),
        ];
        for (s, want) in ok {
            let got = calc(s).unwrap_or_else(|| panic!("{s} -> None"));
            assert!((got - want).abs() < 1e-9, "{s} = {got}, want {want}");
        }
        for s in ["", "1+", "(1", "1)", "2 3", "1/0", "foo(1)", "abc", "1.2.3", "sqrt", "1+*2", "$5", "sin(", &"(".repeat(500)] {
            assert_eq!(calc(s), None, "{s:?} should be None");
        }
        assert_eq!(fmt_num(0.1 + 0.2), "0.3");
        assert_eq!(fmt_num(-0.0), "0");
        assert_eq!(calc_row("42"), None);
        assert!(calc_row("40+2").is_some());
    }

    #[test]
    fn units() {
        let near = |q: &str, want: &str| assert_eq!(convert(q).map(|c| c.2), Some(want.to_string()), "{q}");
        near("1 km in m", "1000 m");
        near("5km to mi", "3.1068559612 mi");
        near("0 c in f", "32 f");
        near("100 C to K", "373.15 K");
        near("-40 f in c", "-40 c");
        near("1,5 h in min", "90 min");
        near("1 gib in mb", "1073.741824 mb");
        for bad in ["5 km in kg", "5 km", "km in mi", "5 foo in m", "5 in in", "hello world", ""] {
            assert!(convert(bad).is_none(), "{bad:?}");
        }
    }
}
