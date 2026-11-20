//! Calculator and unit converter for the search box.
//!   2^10 / 4        sqrt(2)*pi       15% of 80        (3+4)!? no: 5!
//!   10 km to mi     72 f in c        1.5 gb to mb     255 to hex      0xff
use serde::Serialize;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CalcResult {
    /// what to show, e.g. "1,024"
    pub display: String,
    /// what Enter copies, e.g. "1024"
    pub copy: String,
    /// small caption, e.g. "10 km → mi"
    pub caption: String,
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(char),
    LParen,
    RParen,
    Comma,
}

fn lex(s: &str) -> Option<Vec<Tok>> {
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == '0' && i + 1 < c.len() && matches!(c[i + 1], 'x' | 'X' | 'b' | 'B' | 'o' | 'O') {
            let radix = match c[i + 1] {
                'x' | 'X' => 16,
                'b' | 'B' => 2,
                _ => 8,
            };
            let mut j = i + 2;
            let mut digits = String::new();
            while j < c.len() && (c[j].is_digit(radix) || c[j] == '_') {
                if c[j] != '_' {
                    digits.push(c[j]);
                }
                j += 1;
            }
            if digits.is_empty() {
                return None;
            }
            out.push(Tok::Num(i64::from_str_radix(&digits, radix).ok()? as f64));
            i = j;
            continue;
        }
        if ch.is_ascii_digit() || (ch == '.' && i + 1 < c.len() && c[i + 1].is_ascii_digit()) {
            let mut j = i;
            let mut t = String::new();
            while j < c.len() && (c[j].is_ascii_digit() || c[j] == '.' || c[j] == '_') {
                if c[j] != '_' {
                    t.push(c[j]);
                }
                j += 1;
            }
            // exponent: 1e3, 2.5E-4
            if j < c.len() && (c[j] == 'e' || c[j] == 'E') {
                let mut k = j + 1;
                if k < c.len() && (c[k] == '+' || c[k] == '-') {
                    k += 1;
                }
                if k < c.len() && c[k].is_ascii_digit() {
                    t.push('e');
                    t.extend(c[j + 1..k].iter());
                    j = k;
                    while j < c.len() && c[j].is_ascii_digit() {
                        t.push(c[j]);
                        j += 1;
                    }
                }
            }
            out.push(Tok::Num(t.parse().ok()?));
            i = j;
            continue;
        }
        if ch.is_alphabetic() || ch == 'π' || ch == '°' {
            let mut j = i;
            let mut t = String::new();
            while j < c.len() && (c[j].is_alphanumeric() || c[j] == 'π' || c[j] == '°' || c[j] == '/') {
                // "km/h", "m/s" stay one word; a lone "/" is division
                if c[j] == '/' && !(j + 1 < c.len() && c[j + 1].is_alphabetic() && t.len() <= 3) {
                    break;
                }
                t.push(c[j]);
                j += 1;
            }
            out.push(Tok::Ident(t.to_lowercase()));
            i = j;
            continue;
        }
        let op = match ch {
            '+' | '-' | '*' | '/' | '^' | '%' | '!' => ch,
            '×' => '*',
            '÷' => '/',
            '−' => '-',
            '(' => {
                out.push(Tok::LParen);
                i += 1;
                continue;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
                continue;
            }
            ',' => {
                out.push(Tok::Comma);
                i += 1;
                continue;
            }
            _ => return None,
        };
        // "**" is power
        if op == '*' && i + 1 < c.len() && c[i + 1] == '*' {
            out.push(Tok::Op('^'));
            i += 2;
            continue;
        }
        out.push(Tok::Op(op));
        i += 1;
    }
    Some(out)
}

struct P {
    t: Vec<Tok>,
    i: usize,
    ops: usize,
}

impl P {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.t.get(self.i).cloned();
        self.i += 1;
        t
    }
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        while let Some(Tok::Op(o @ ('+' | '-'))) = self.peek().cloned() {
            self.i += 1;
            self.ops += 1;
            let r = self.term()?;
            v = if o == '+' { v + r } else { v - r };
        }
        Some(v)
    }
    fn term(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        loop {
            match self.peek().cloned() {
                Some(Tok::Op(o @ ('*' | '/'))) => {
                    self.i += 1;
                    self.ops += 1;
                    let r = self.unary()?;
                    v = if o == '*' { v * r } else { v / r };
                }
                Some(Tok::Op('%')) => {
                    // "a % b" is modulo; "a%" (end, operator, "of") is percent
                    let next = self.t.get(self.i + 1).cloned();
                    match next {
                        Some(Tok::Num(_)) | Some(Tok::LParen) => {
                            self.i += 1;
                            self.ops += 1;
                            let r = self.unary()?;
                            v %= r;
                        }
                        _ => {
                            self.i += 1;
                            self.ops += 1;
                            v /= 100.0;
                        }
                    }
                }
                Some(Tok::Ident(w)) if w == "of" || w == "mod" || w == "x" => {
                    self.i += 1;
                    self.ops += 1;
                    let r = self.unary()?;
                    v = if w == "mod" { v % r } else { v * r };
                }
                // implicit multiplication: 2pi, 3(4+1), 2 sqrt(9)
                Some(Tok::LParen) | Some(Tok::Num(_)) => {
                    self.ops += 1;
                    let r = self.unary()?;
                    v *= r;
                }
                Some(Tok::Ident(w)) if is_func(&w) || is_const(&w) => {
                    self.ops += 1;
                    let r = self.unary()?;
                    v *= r;
                }
                _ => return Some(v),
            }
        }
    }
    fn unary(&mut self) -> Option<f64> {
        match self.peek() {
            Some(Tok::Op('-')) => {
                self.i += 1;
                Some(-self.unary()?)
            }
            Some(Tok::Op('+')) => {
                self.i += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }
    fn power(&mut self) -> Option<f64> {
        let base = self.postfix()?;
        if let Some(Tok::Op('^')) = self.peek() {
            self.i += 1;
            self.ops += 1;
            let e = self.unary()?;
            return Some(base.powf(e));
        }
        Some(base)
    }
    fn postfix(&mut self) -> Option<f64> {
        let mut v = self.primary()?;
        while let Some(Tok::Op('!')) = self.peek() {
            self.i += 1;
            self.ops += 1;
            if v < 0.0 || v.fract() != 0.0 || v > 170.0 {
                return None;
            }
            v = (1..=v as u64).fold(1.0, |a, b| a * b as f64);
        }
        Some(v)
    }
    fn primary(&mut self) -> Option<f64> {
        match self.next()? {
            Tok::Num(n) => Some(n),
            Tok::LParen => {
                let v = self.expr()?;
                if let Some(Tok::RParen) = self.peek() {
                    self.i += 1;
                }
                Some(v)
            }
            Tok::Ident(w) => {
                if let Some(c) = constant(&w) {
                    return Some(c);
                }
                if !is_func(&w) {
                    return None;
                }
                self.ops += 1;
                let args = if let Some(Tok::LParen) = self.peek() {
                    self.i += 1;
                    let mut a = vec![self.expr()?];
                    while let Some(Tok::Comma) = self.peek() {
                        self.i += 1;
                        a.push(self.expr()?);
                    }
                    if let Some(Tok::RParen) = self.peek() {
                        self.i += 1;
                    }
                    a
                } else {
                    vec![self.power()?]
                };
                func(&w, &args)
            }
            _ => None,
        }
    }
}

fn is_const(w: &str) -> bool {
    constant(w).is_some()
}

fn constant(w: &str) -> Option<f64> {
    Some(match w {
        "pi" | "π" => std::f64::consts::PI,
        "e" => std::f64::consts::E,
        "tau" => std::f64::consts::TAU,
        "phi" => 1.618_033_988_749_895,
        _ => return None,
    })
}

const FUNCS: &[&str] = &[
    "sqrt", "cbrt", "abs", "round", "floor", "ceil", "ln", "log", "log2", "log10", "exp", "sin", "cos", "tan", "asin", "acos", "atan", "sinh", "cosh",
    "tanh", "min", "max", "pow", "avg", "sum", "deg", "rad",
];

fn is_func(w: &str) -> bool {
    FUNCS.contains(&w)
}

fn func(w: &str, a: &[f64]) -> Option<f64> {
    let x = *a.first()?;
    Some(match w {
        "sqrt" => x.sqrt(),
        "cbrt" => x.cbrt(),
        "abs" => x.abs(),
        "round" => {
            if let Some(d) = a.get(1) {
                let m = 10f64.powi(*d as i32);
                (x * m).round() / m
            } else {
                x.round()
            }
        }
        "floor" => x.floor(),
        "ceil" => x.ceil(),
        "ln" => x.ln(),
        "log" | "log10" => {
            if let Some(b) = a.get(1) {
                x.ln() / b.ln()
            } else {
                x.log10()
            }
        }
        "log2" => x.log2(),
        "exp" => x.exp(),
        "sin" => x.sin(),
        "cos" => x.cos(),
        "tan" => x.tan(),
        "asin" => x.asin(),
        "acos" => x.acos(),
        "atan" => x.atan(),
        "sinh" => x.sinh(),
        "cosh" => x.cosh(),
        "tanh" => x.tanh(),
        "deg" => x.to_degrees(),
        "rad" => x.to_radians(),
        "min" => a.iter().cloned().fold(f64::INFINITY, f64::min),
        "max" => a.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        "sum" => a.iter().sum(),
        "avg" => a.iter().sum::<f64>() / a.len() as f64,
        "pow" => x.powf(*a.get(1)?),
        _ => return None,
    })
}

/// Evaluate; also returns how many operations it had (a bare "42" has none).
fn eval(s: &str) -> Option<(f64, usize)> {
    let t = lex(s)?;
    if t.is_empty() {
        return None;
    }
    let mut p = P { t, i: 0, ops: 0 };
    let v = p.expr()?;
    // allow a trailing ")" or "=" leftovers only
    while let Some(Tok::RParen) = p.peek() {
        p.i += 1;
    }
    if p.i != p.t.len() || !v.is_finite() {
        return None;
    }
    Some((v, p.ops))
}

// ------------------------------------------------------------------------------------------
// units

#[derive(Clone, Copy, PartialEq, Debug)]
enum Dim {
    Length,
    Mass,
    Temp,
    Data,
    Time,
    Speed,
    Volume,
    Area,
}

/// (names, dimension, factor to the base unit)
const UNITS: &[(&[&str], Dim, f64)] = &[
    (&["mm", "millimeter", "millimeters"], Dim::Length, 0.001),
    (&["cm", "centimeter", "centimeters"], Dim::Length, 0.01),
    (&["m", "meter", "meters", "metre", "metres"], Dim::Length, 1.0),
    (&["km", "kilometer", "kilometers"], Dim::Length, 1000.0),
    (&["in", "inch", "inches", "\""], Dim::Length, 0.0254),
    (&["ft", "foot", "feet"], Dim::Length, 0.3048),
    (&["yd", "yard", "yards"], Dim::Length, 0.9144),
    (&["mi", "mile", "miles"], Dim::Length, 1609.344),
    (&["nmi"], Dim::Length, 1852.0),
    (&["mg", "milligram", "milligrams"], Dim::Mass, 0.001),
    (&["g", "gram", "grams"], Dim::Mass, 1.0),
    (&["kg", "kilo", "kilos", "kilogram", "kilograms"], Dim::Mass, 1000.0),
    (&["t", "tonne", "tonnes"], Dim::Mass, 1_000_000.0),
    (&["oz", "ounce", "ounces"], Dim::Mass, 28.349_523_125),
    (&["lb", "lbs", "pound", "pounds"], Dim::Mass, 453.592_37),
    (&["st", "stone"], Dim::Mass, 6350.293_18),
    (&["c", "°c", "celsius"], Dim::Temp, 0.0),
    (&["f", "°f", "fahrenheit"], Dim::Temp, 1.0),
    (&["k", "kelvin"], Dim::Temp, 2.0),
    (&["b", "byte", "bytes"], Dim::Data, 1.0),
    (&["kb", "kilobyte", "kilobytes"], Dim::Data, 1e3),
    (&["mb", "megabyte", "megabytes"], Dim::Data, 1e6),
    (&["gb", "gigabyte", "gigabytes"], Dim::Data, 1e9),
    (&["tb", "terabyte", "terabytes"], Dim::Data, 1e12),
    (&["kib"], Dim::Data, 1024.0),
    (&["mib"], Dim::Data, 1_048_576.0),
    (&["gib"], Dim::Data, 1_073_741_824.0),
    (&["tib"], Dim::Data, 1_099_511_627_776.0),
    (&["bit", "bits"], Dim::Data, 0.125),
    (&["ms", "millisecond", "milliseconds"], Dim::Time, 0.001),
    (&["s", "sec", "secs", "second", "seconds"], Dim::Time, 1.0),
    (&["min", "mins", "minute", "minutes"], Dim::Time, 60.0),
    (&["h", "hr", "hrs", "hour", "hours"], Dim::Time, 3600.0),
    (&["d", "day", "days"], Dim::Time, 86400.0),
    (&["wk", "week", "weeks"], Dim::Time, 604_800.0),
    (&["yr", "year", "years"], Dim::Time, 31_557_600.0),
    (&["m/s", "mps"], Dim::Speed, 1.0),
    (&["km/h", "kmh", "kph"], Dim::Speed, 1.0 / 3.6),
    (&["mph"], Dim::Speed, 0.44704),
    (&["kn", "knot", "knots"], Dim::Speed, 0.514_444),
    (&["ml", "milliliter", "milliliters"], Dim::Volume, 0.001),
    (&["l", "liter", "liters", "litre", "litres"], Dim::Volume, 1.0),
    (&["gal", "gallon", "gallons"], Dim::Volume, 3.785_411_784),
    (&["floz"], Dim::Volume, 0.029_573_53),
    (&["cup", "cups"], Dim::Volume, 0.236_588_2),
    (&["m2", "sqm"], Dim::Area, 1.0),
    (&["km2"], Dim::Area, 1e6),
    (&["ft2", "sqft"], Dim::Area, 0.092_903_04),
    (&["acre", "acres"], Dim::Area, 4046.856_422_4),
    (&["ha", "hectare", "hectares"], Dim::Area, 10_000.0),
];

fn unit(name: &str) -> Option<(Dim, f64, &'static str)> {
    UNITS.iter().find(|(n, _, _)| n.contains(&name)).map(|(n, d, f)| (*d, *f, n[0]))
}

fn temp_to_c(v: f64, code: f64) -> f64 {
    match code as i32 {
        0 => v,
        1 => (v - 32.0) * 5.0 / 9.0,
        _ => v - 273.15,
    }
}
fn c_to_temp(c: f64, code: f64) -> f64 {
    match code as i32 {
        0 => c,
        1 => c * 9.0 / 5.0 + 32.0,
        _ => c + 273.15,
    }
}

/// Number formatting: up to 10 significant decimals, thousands grouped for display.
pub fn fmt_num(v: f64) -> (String, String) {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        let raw = format!("{}", v as i64);
        return (group(&raw), raw);
    }
    if v.abs() >= 1e15 || (v != 0.0 && v.abs() < 1e-6) {
        let s = format!("{v:.6e}");
        return (s.clone(), s);
    }
    let mut raw = format!("{v:.10}");
    while raw.ends_with('0') {
        raw.pop();
    }
    if raw.ends_with('.') {
        raw.pop();
    }
    let (int, frac) = raw.split_once('.').map(|(a, b)| (a.to_string(), format!(".{b}"))).unwrap_or((raw.clone(), String::new()));
    (format!("{}{}", group(&int), frac), raw)
}

fn group(int: &str) -> String {
    let (sign, digits) = if let Some(d) = int.strip_prefix('-') { ("-", d) } else { ("", int) };
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{sign}{out}")
}

fn convert(input: &str) -> Option<CalcResult> {
    let lower = input.to_lowercase();
    let (left, target) = [" to ", " in ", " as ", " -> ", " => "].iter().find_map(|sep| lower.rsplit_once(sep))?;
    let target = target.trim();
    let left = left.trim();
    // radix: "255 to hex"
    if matches!(target, "hex" | "bin" | "binary" | "oct" | "octal" | "dec" | "decimal") {
        let (v, _) = eval(left)?;
        if v.fract() != 0.0 || v.abs() > 9.0e15 {
            return None;
        }
        let n = v as i64;
        let s = match target {
            "hex" => format!("0x{n:X}"),
            "bin" | "binary" => format!("0b{n:b}"),
            "oct" | "octal" => format!("0o{n:o}"),
            _ => n.to_string(),
        };
        return Some(CalcResult { display: s.clone(), copy: s, caption: format!("{left} → {target}") });
    }
    let (tdim, tf, tname) = unit(target)?;
    // split "10.5km" / "10 km" / "2*5 km" into amount and unit: the unit is the trailing word
    let split_at = left.rfind(|c: char| c.is_ascii_digit() || c == ')' || c == ' ')?;
    let (amount, uname) = left.split_at(split_at + 1);
    let uname = uname.trim();
    let (amount, uname) = if uname.is_empty() {
        // "10 km": last word is the unit
        let (a, u) = left.rsplit_once(' ')?;
        (a.trim(), u.trim())
    } else {
        (amount.trim(), uname)
    };
    let (sdim, sf, sname) = unit(uname)?;
    if sdim != tdim {
        return None;
    }
    let (v, _) = eval(amount)?;
    let out = if sdim == Dim::Temp { c_to_temp(temp_to_c(v, sf), tf) } else { v * sf / tf };
    let (d, c) = fmt_num(out);
    let (dv, _) = fmt_num(v);
    Some(CalcResult { display: format!("{d} {tname}"), copy: c, caption: format!("{dv} {sname} → {tname}") })
}

/// A result if `input` looks like math or a conversion.
pub fn evaluate(input: &str) -> Option<CalcResult> {
    let s = input.trim().trim_start_matches('=').trim();
    if s.is_empty() || !s.chars().any(|c| c.is_ascii_digit() || c == 'π') {
        return None;
    }
    if let Some(r) = convert(s) {
        return Some(r);
    }
    let s = s.trim_end_matches('=').trim();
    let (v, ops) = eval(s)?;
    let is_radix_literal = s.starts_with("0x") || s.starts_with("0b") || s.starts_with("0o");
    if ops == 0 && !is_radix_literal && !input.trim_start().starts_with('=') {
        return None;
    }
    let (display, copy) = fmt_num(v);
    Some(CalcResult { display, copy, caption: s.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> String {
        evaluate(s).map(|r| r.copy).unwrap_or_else(|| "none".into())
    }

    #[test]
    fn arithmetic() {
        assert_eq!(v("2+3*4"), "14");
        assert_eq!(v("(2+3)*4"), "20");
        assert_eq!(v("2^10"), "1024");
        assert_eq!(v("2**3"), "8");
        assert_eq!(v("-3^2"), "-9");
        assert_eq!(v("10/4"), "2.5");
        assert_eq!(v("15% of 80"), "12");
        assert_eq!(v("10 % 3"), "1");
        assert_eq!(v("5!"), "120");
        assert_eq!(v("2pi"), "6.2831853072");
        assert_eq!(v("sqrt(16)+abs(-2)"), "6");
        assert_eq!(v("max(3, 9, 2)"), "9");
        assert_eq!(v("1e3*2"), "2000");
        assert_eq!(v("0xff"), "255");
        assert_eq!(v("3 × 4"), "12");
    }

    #[test]
    fn not_math() {
        assert_eq!(v("42"), "none");
        assert_eq!(v("7zip"), "none");
        assert_eq!(v("chrome"), "none");
        assert_eq!(v("windows 11"), "none");
        assert_eq!(v("1/0"), "none");
        assert_eq!(v("=42"), "42");
    }

    #[test]
    fn conversions() {
        assert_eq!(v("10 km to mi"), "6.2137119224");
        assert_eq!(v("212 f in c"), "100");
        assert_eq!(v("1.5 gb to mb"), "1500");
        assert_eq!(v("1gib in mib"), "1024");
        assert_eq!(v("255 to hex"), "0xFF");
        assert_eq!(v("0b1010 to dec"), "10");
        assert_eq!(v("100 km/h to mph"), "62.1371192237");
        assert_eq!(v("10 kg to km"), "none");
        assert_eq!(evaluate("1234567.5").map(|r| r.display), None);
        assert_eq!(evaluate("1000*1234.5").unwrap().display, "1,234,500");
    }
}
