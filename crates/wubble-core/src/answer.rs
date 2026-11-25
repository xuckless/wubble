//! Command-bar answers computed on the device: a calculator and unit conversion.
//!
//! Chrome's omnibox answers `12*7+3`, `sqrt(2)`, `5 km in miles` and `70 F to C` with a row
//! above the search, fetched from Google's server. wubble computes them here instead, with no
//! request: [`answer`] parses the query, and `omnibox::rank` places the row. Pure and
//! deterministic. Written new (a small recursive-descent parser); the conversion factors are
//! the exact SI definitions (NIST SP 811 appendix B). No currency: it needs a rate source.

/// What kind of answer a query gives; it decides the row's place (`omnibox::rank`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerKind {
    /// An expression with at least one operator: the row goes first.
    Math,
    /// `<n> <unit> in|to <unit>`: the row goes under the search intent.
    Unit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub kind: AnswerKind,
    /// What the row shows: `12*7+3 = 87`, `5 km = 3.10686 mi`.
    pub title: String,
    /// What choosing the row copies: `87`, `3.10686 mi`.
    pub copy: String,
    pub value: f64,
}

/// Significant digits a calculator result keeps (hides float noise: 0.1 + 0.2 = 0.3).
const MATH_DIGITS: i32 = 12;
/// Significant digits a converted value keeps.
const UNIT_DIGITS: i32 = 6;

/// The answer `query` gives, if any: a unit conversion, else a calculation. A bare number, an
/// expression with no operator (`pi`), anything that does not parse and a result that is not
/// finite (division by zero) give none.
pub fn answer(query: &str) -> Option<Answer> {
    let q = query.trim();
    if q.is_empty() {
        return None;
    }
    convert(q).or_else(|| calculate(q))
}

// ------------------------------------------------------------- calculator

fn calculate(q: &str) -> Option<Answer> {
    let (value, ops) = evaluate(q)?;
    if ops == 0 {
        return None;
    }
    let shown = format_number(value, MATH_DIGITS);
    Some(Answer {
        kind: AnswerKind::Math,
        title: format!("{q} = {shown}"),
        copy: shown,
        value,
    })
}

/// The value of `expr` and how many operators it applied; `None` when it does not parse or the
/// value is not finite.
pub fn evaluate(expr: &str) -> Option<(f64, usize)> {
    let tokens = tokenize(expr)?;
    let mut p = Parser {
        tokens,
        at: 0,
        ops: 0,
    };
    let (value, _) = p.expr()?;
    if p.at != p.tokens.len() || !value.is_finite() {
        return None;
    }
    Some((value, p.ops))
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Percent,
    Open,
    Close,
}

fn tokenize(s: &str) -> Option<Vec<Tok>> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let tok = match c {
            c if c.is_whitespace() => {
                i += 1;
                continue;
            }
            '0'..='9' | '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                // An exponent only when digits follow: `2e3`; `2e` is 2 times e.
                if i < chars.len() && matches!(chars[i], 'e' | 'E') {
                    let mut j = i + 1;
                    if j < chars.len() && matches!(chars[j], '+' | '-') {
                        j += 1;
                    }
                    if j < chars.len() && chars[j].is_ascii_digit() {
                        while j < chars.len() && chars[j].is_ascii_digit() {
                            j += 1;
                        }
                        i = j;
                    }
                }
                let text: String = chars[start..i].iter().collect();
                out.push(Tok::Num(text.parse().ok()?));
                continue;
            }
            c if c.is_alphabetic() => {
                let start = i;
                while i < chars.len() && chars[i].is_alphabetic() {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                out.push(Tok::Ident(text.to_lowercase()));
                continue;
            }
            '+' => Tok::Plus,
            '-' | '\u{2212}' => Tok::Minus,
            '*' | '\u{d7}' | '\u{b7}' => Tok::Star,
            '/' | '\u{f7}' => Tok::Slash,
            '^' => Tok::Caret,
            '%' => Tok::Percent,
            '(' => Tok::Open,
            ')' => Tok::Close,
            _ => return None,
        };
        out.push(tok);
        i += 1;
    }
    Some(out)
}

type Func = fn(f64) -> f64;

/// The functions, as data: name, what it does.
const FUNCTIONS: [(&str, Func); 8] = [
    ("sqrt", f64::sqrt),
    ("sin", f64::sin),
    ("cos", f64::cos),
    ("tan", f64::tan),
    ("log", f64::log10),
    ("ln", f64::ln),
    ("abs", f64::abs),
    ("round", f64::round),
];

const CONSTANTS: [(&str, f64); 3] = [
    ("pi", std::f64::consts::PI),
    ("\u{3c0}", std::f64::consts::PI),
    ("e", std::f64::consts::E),
];

/// Recursive descent, lowest precedence first:
/// expr = term (("+" | "-") term)*
/// term = unary (("*" | "/" | "%") unary | implicit unary)*
/// unary = ("-" | "+") unary | power
/// power = postfix ("^" unary)?        (right-associative; -2^2 = -4)
/// postfix = primary ("%" ("of" unary)?)*
/// primary = number | constant | function arg | "(" expr ")"
struct Parser {
    tokens: Vec<Tok>,
    at: usize,
    /// Operators applied: a query with none is not a calculation.
    ops: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.at)
    }

    fn peek_at(&self, n: usize) -> Option<&Tok> {
        self.tokens.get(self.at + n)
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == Some(t) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    /// Whether the token at `n` ahead starts an operand (for `%`: modulo, not percent).
    fn operand_at(&self, n: usize) -> bool {
        match self.peek_at(n) {
            Some(Tok::Num(_) | Tok::Open) => true,
            Some(Tok::Ident(id)) => id != "of",
            _ => false,
        }
    }

    /// The value, and whether it is a bare percentage (`10%`): `100 + 10%` is 110, as on
    /// Google's calculator.
    fn expr(&mut self) -> Option<(f64, bool)> {
        let (mut value, mut pct) = self.term()?;
        loop {
            let sign = if self.eat(&Tok::Plus) {
                1.0
            } else if self.eat(&Tok::Minus) {
                -1.0
            } else {
                return Some((value, pct));
            };
            self.ops += 1;
            let (rhs, rhs_pct) = self.term()?;
            value = if rhs_pct {
                value * (1.0 + sign * rhs)
            } else {
                value + sign * rhs
            };
            pct = false;
        }
    }

    fn term(&mut self) -> Option<(f64, bool)> {
        let (mut value, mut pct) = self.unary()?;
        loop {
            match self.peek().cloned() {
                Some(Tok::Star) => {
                    self.at += 1;
                    value *= self.unary()?.0;
                }
                Some(Tok::Slash) => {
                    self.at += 1;
                    let d = self.unary()?.0;
                    if d == 0.0 {
                        return None;
                    }
                    value /= d;
                }
                // `%` between operands is the remainder (a trailing one was taken as percent).
                Some(Tok::Percent) if self.operand_at(1) => {
                    self.at += 1;
                    let d = self.unary()?.0;
                    if d == 0.0 {
                        return None;
                    }
                    value %= d;
                }
                // Implicit multiplication: `2pi`, `3(4+1)`, `2 sqrt 9`.
                Some(Tok::Open | Tok::Ident(_)) if self.operand_at(0) => {
                    value *= self.unary()?.0;
                }
                _ => return Some((value, pct)),
            }
            self.ops += 1;
            pct = false;
        }
    }

    fn unary(&mut self) -> Option<(f64, bool)> {
        if self.eat(&Tok::Minus) {
            // A negative number is still only a number; `-(2)` and `-pi` count.
            if !matches!(self.peek(), Some(Tok::Num(_))) {
                self.ops += 1;
            }
            let (v, pct) = self.unary()?;
            return Some((-v, pct));
        }
        if self.eat(&Tok::Plus) {
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Option<(f64, bool)> {
        let (base, pct) = self.postfix()?;
        if self.eat(&Tok::Caret) {
            self.ops += 1;
            let exp = self.unary()?.0;
            return Some((base.powf(exp), false));
        }
        Some((base, pct))
    }

    fn postfix(&mut self) -> Option<(f64, bool)> {
        let mut value = self.primary()?;
        let mut pct = false;
        while self.peek() == Some(&Tok::Percent) && !self.operand_at(1) {
            self.at += 1;
            self.ops += 1;
            value /= 100.0;
            pct = true;
            // `15% of 80`
            if self.peek() == Some(&Tok::Ident("of".into())) {
                self.at += 1;
                value *= self.unary()?.0;
                pct = false;
            }
        }
        Some((value, pct))
    }

    fn primary(&mut self) -> Option<f64> {
        match self.peek()?.clone() {
            Tok::Num(n) => {
                self.at += 1;
                Some(n)
            }
            Tok::Open => {
                self.at += 1;
                let (v, _) = self.expr()?;
                // A missing `)` at the end closes itself, as Google's calculator allows.
                if !self.eat(&Tok::Close) && self.peek().is_some() {
                    return None;
                }
                Some(v)
            }
            Tok::Ident(id) => {
                self.at += 1;
                if let Some((_, c)) = CONSTANTS.iter().find(|(n, _)| *n == id) {
                    return Some(*c);
                }
                let (_, f) = FUNCTIONS.iter().find(|(n, _)| *n == id)?;
                self.ops += 1;
                let arg = self.unary()?.0;
                Some(f(arg))
            }
            _ => None,
        }
    }
}

// ------------------------------------------------------------------ units

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dim {
    Length,
    Mass,
    Volume,
    Temperature,
    Speed,
    Area,
    Data,
    Time,
}

/// One unit as data. `base = (value + offset) * factor`, the base unit being the family's SI
/// one (metre, kilogram, litre, kelvin, metre per second, square metre, byte, second).
#[derive(Debug, PartialEq)]
pub struct Unit {
    pub dim: Dim,
    /// What the answer shows.
    pub symbol: &'static str,
    /// What may be typed, lower case, the symbol among them.
    pub names: &'static [&'static str],
    pub factor: f64,
    pub offset: f64,
}

const fn u(dim: Dim, symbol: &'static str, names: &'static [&'static str], factor: f64) -> Unit {
    Unit {
        dim,
        symbol,
        names,
        factor,
        offset: 0.0,
    }
}

const DAY: f64 = 86_400.0;
const INCH: f64 = 0.0254;
const FOOT: f64 = 0.3048;
const YARD: f64 = 0.9144;
const MILE: f64 = 1_609.344;
const US_GALLON: f64 = 3.785_411_784;

/// Every unit, by family. Names are matched lower case, so `MB` and `Mb` both name the
/// megabyte; the bit forms are spelled out (`mbit`, `megabit`).
pub const UNITS: &[Unit] = &[
    // length (metre)
    u(
        Dim::Length,
        "nm",
        &["nm", "nanometer", "nanometers", "nanometre", "nanometres"],
        1e-9,
    ),
    u(
        Dim::Length,
        "\u{b5}m",
        &[
            "\u{b5}m",
            "um",
            "micrometer",
            "micrometers",
            "micron",
            "microns",
        ],
        1e-6,
    ),
    u(
        Dim::Length,
        "mm",
        &[
            "mm",
            "millimeter",
            "millimeters",
            "millimetre",
            "millimetres",
        ],
        1e-3,
    ),
    u(
        Dim::Length,
        "cm",
        &[
            "cm",
            "centimeter",
            "centimeters",
            "centimetre",
            "centimetres",
        ],
        1e-2,
    ),
    u(
        Dim::Length,
        "m",
        &["m", "meter", "meters", "metre", "metres"],
        1.0,
    ),
    u(
        Dim::Length,
        "km",
        &["km", "kilometer", "kilometers", "kilometre", "kilometres"],
        1e3,
    ),
    u(Dim::Length, "in", &["in", "inch", "inches", "\""], INCH),
    u(Dim::Length, "ft", &["ft", "foot", "feet", "'"], FOOT),
    u(Dim::Length, "yd", &["yd", "yard", "yards"], YARD),
    u(Dim::Length, "mi", &["mi", "mile", "miles"], MILE),
    u(
        Dim::Length,
        "nmi",
        &["nmi", "nautical mile", "nautical miles"],
        1_852.0,
    ),
    // mass (kilogram)
    u(Dim::Mass, "mg", &["mg", "milligram", "milligrams"], 1e-6),
    u(
        Dim::Mass,
        "g",
        &["g", "gram", "grams", "gramme", "grammes"],
        1e-3,
    ),
    u(
        Dim::Mass,
        "kg",
        &["kg", "kilo", "kilos", "kilogram", "kilograms"],
        1.0,
    ),
    u(
        Dim::Mass,
        "t",
        &["t", "tonne", "tonnes", "metric ton", "metric tons"],
        1e3,
    ),
    u(
        Dim::Mass,
        "oz",
        &["oz", "ounce", "ounces"],
        0.028_349_523_125,
    ),
    u(
        Dim::Mass,
        "lb",
        &["lb", "lbs", "pound", "pounds"],
        0.453_592_37,
    ),
    u(Dim::Mass, "st", &["st", "stone", "stones"], 6.350_293_18),
    u(
        Dim::Mass,
        "ton",
        &["ton", "tons", "short ton", "short tons"],
        907.184_74,
    ),
    // volume (litre)
    u(
        Dim::Volume,
        "ml",
        &[
            "ml",
            "milliliter",
            "milliliters",
            "millilitre",
            "millilitres",
            "cc",
            "cm3",
            "cm\u{b3}",
        ],
        1e-3,
    ),
    u(
        Dim::Volume,
        "cl",
        &[
            "cl",
            "centiliter",
            "centiliters",
            "centilitre",
            "centilitres",
        ],
        1e-2,
    ),
    u(
        Dim::Volume,
        "dl",
        &["dl", "deciliter", "deciliters", "decilitre", "decilitres"],
        1e-1,
    ),
    u(
        Dim::Volume,
        "l",
        &["l", "liter", "liters", "litre", "litres"],
        1.0,
    ),
    u(
        Dim::Volume,
        "m\u{b3}",
        &[
            "m3",
            "m\u{b3}",
            "cubic meter",
            "cubic meters",
            "cubic metre",
            "cubic metres",
        ],
        1e3,
    ),
    u(
        Dim::Volume,
        "tsp",
        &["tsp", "teaspoon", "teaspoons"],
        US_GALLON / 768.0,
    ),
    u(
        Dim::Volume,
        "tbsp",
        &["tbsp", "tablespoon", "tablespoons"],
        US_GALLON / 256.0,
    ),
    u(
        Dim::Volume,
        "fl oz",
        &["fl oz", "floz", "fluid ounce", "fluid ounces"],
        US_GALLON / 128.0,
    ),
    u(Dim::Volume, "cup", &["cup", "cups"], US_GALLON / 16.0),
    u(Dim::Volume, "pt", &["pt", "pint", "pints"], US_GALLON / 8.0),
    u(
        Dim::Volume,
        "qt",
        &["qt", "quart", "quarts"],
        US_GALLON / 4.0,
    ),
    u(Dim::Volume, "gal", &["gal", "gallon", "gallons"], US_GALLON),
    u(
        Dim::Volume,
        "imp gal",
        &["imp gal", "imperial gallon", "imperial gallons"],
        4.546_09,
    ),
    // temperature (kelvin)
    Unit {
        dim: Dim::Temperature,
        symbol: "\u{b0}C",
        names: &[
            "c",
            "celsius",
            "centigrade",
            "degc",
            "degree celsius",
            "degrees celsius",
        ],
        factor: 1.0,
        offset: 273.15,
    },
    Unit {
        dim: Dim::Temperature,
        symbol: "\u{b0}F",
        names: &[
            "f",
            "fahrenheit",
            "degf",
            "degree fahrenheit",
            "degrees fahrenheit",
        ],
        factor: 5.0 / 9.0,
        offset: 459.67,
    },
    u(Dim::Temperature, "K", &["k", "kelvin", "kelvins"], 1.0),
    // speed (metre per second)
    u(
        Dim::Speed,
        "m/s",
        &[
            "m/s",
            "mps",
            "meter per second",
            "meters per second",
            "metre per second",
            "metres per second",
        ],
        1.0,
    ),
    u(
        Dim::Speed,
        "km/h",
        &[
            "km/h",
            "kmh",
            "kph",
            "kmph",
            "kilometer per hour",
            "kilometers per hour",
            "kilometre per hour",
            "kilometres per hour",
        ],
        1_000.0 / 3_600.0,
    ),
    u(
        Dim::Speed,
        "mph",
        &["mph", "mi/h", "mile per hour", "miles per hour"],
        MILE / 3_600.0,
    ),
    u(
        Dim::Speed,
        "ft/s",
        &["ft/s", "fps", "foot per second", "feet per second"],
        FOOT,
    ),
    u(
        Dim::Speed,
        "kn",
        &["kn", "kt", "kts", "knot", "knots"],
        1_852.0 / 3_600.0,
    ),
    // area (square metre)
    u(
        Dim::Area,
        "mm\u{b2}",
        &[
            "mm2",
            "mm\u{b2}",
            "sq mm",
            "square millimeter",
            "square millimeters",
            "square millimetre",
            "square millimetres",
        ],
        1e-6,
    ),
    u(
        Dim::Area,
        "cm\u{b2}",
        &[
            "cm2",
            "cm\u{b2}",
            "sq cm",
            "square centimeter",
            "square centimeters",
            "square centimetre",
            "square centimetres",
        ],
        1e-4,
    ),
    u(
        Dim::Area,
        "m\u{b2}",
        &[
            "m2",
            "m\u{b2}",
            "sq m",
            "sqm",
            "square meter",
            "square meters",
            "square metre",
            "square metres",
        ],
        1.0,
    ),
    u(Dim::Area, "ha", &["ha", "hectare", "hectares"], 1e4),
    u(
        Dim::Area,
        "km\u{b2}",
        &[
            "km2",
            "km\u{b2}",
            "sq km",
            "square kilometer",
            "square kilometers",
            "square kilometre",
            "square kilometres",
        ],
        1e6,
    ),
    u(
        Dim::Area,
        "in\u{b2}",
        &["in2", "in\u{b2}", "sq in", "square inch", "square inches"],
        INCH * INCH,
    ),
    u(
        Dim::Area,
        "ft\u{b2}",
        &[
            "ft2",
            "ft\u{b2}",
            "sq ft",
            "sqft",
            "square foot",
            "square feet",
        ],
        FOOT * FOOT,
    ),
    u(
        Dim::Area,
        "yd\u{b2}",
        &["yd2", "yd\u{b2}", "sq yd", "square yard", "square yards"],
        YARD * YARD,
    ),
    u(Dim::Area, "ac", &["ac", "acre", "acres"], 4_046.856_422_4),
    u(
        Dim::Area,
        "mi\u{b2}",
        &["mi2", "mi\u{b2}", "sq mi", "square mile", "square miles"],
        MILE * MILE,
    ),
    // data size (byte)
    u(Dim::Data, "bit", &["bit", "bits"], 0.125),
    u(Dim::Data, "B", &["b", "byte", "bytes"], 1.0),
    u(
        Dim::Data,
        "kbit",
        &["kbit", "kbits", "kilobit", "kilobits"],
        125.0,
    ),
    u(
        Dim::Data,
        "Mbit",
        &["mbit", "mbits", "megabit", "megabits"],
        125e3,
    ),
    u(
        Dim::Data,
        "Gbit",
        &["gbit", "gbits", "gigabit", "gigabits"],
        125e6,
    ),
    u(Dim::Data, "kB", &["kb", "kilobyte", "kilobytes"], 1e3),
    u(Dim::Data, "MB", &["mb", "megabyte", "megabytes"], 1e6),
    u(Dim::Data, "GB", &["gb", "gigabyte", "gigabytes"], 1e9),
    u(Dim::Data, "TB", &["tb", "terabyte", "terabytes"], 1e12),
    u(Dim::Data, "PB", &["pb", "petabyte", "petabytes"], 1e15),
    u(Dim::Data, "KiB", &["kib", "kibibyte", "kibibytes"], 1_024.0),
    u(
        Dim::Data,
        "MiB",
        &["mib", "mebibyte", "mebibytes"],
        1_048_576.0,
    ),
    u(
        Dim::Data,
        "GiB",
        &["gib", "gibibyte", "gibibytes"],
        1_073_741_824.0,
    ),
    u(
        Dim::Data,
        "TiB",
        &["tib", "tebibyte", "tebibytes"],
        1_099_511_627_776.0,
    ),
    // time (second)
    u(Dim::Time, "ns", &["ns", "nanosecond", "nanoseconds"], 1e-9),
    u(
        Dim::Time,
        "\u{b5}s",
        &["\u{b5}s", "us", "microsecond", "microseconds"],
        1e-6,
    ),
    u(
        Dim::Time,
        "ms",
        &["ms", "millisecond", "milliseconds"],
        1e-3,
    ),
    u(
        Dim::Time,
        "s",
        &["s", "sec", "secs", "second", "seconds"],
        1.0,
    ),
    u(
        Dim::Time,
        "min",
        &["min", "mins", "minute", "minutes"],
        60.0,
    ),
    u(
        Dim::Time,
        "h",
        &["h", "hr", "hrs", "hour", "hours"],
        3_600.0,
    ),
    u(Dim::Time, "d", &["d", "day", "days"], DAY),
    u(Dim::Time, "wk", &["wk", "wks", "week", "weeks"], 7.0 * DAY),
    // the Gregorian averages: 365.2425 days, a twelfth of it
    u(
        Dim::Time,
        "mo",
        &["mo", "month", "months"],
        365.2425 * DAY / 12.0,
    ),
    u(
        Dim::Time,
        "yr",
        &["yr", "yrs", "year", "years"],
        365.2425 * DAY,
    ),
];

/// The unit `name` names (lower case, a leading degree sign and `degree(s)` dropped).
pub fn unit(name: &str) -> Option<&'static Unit> {
    let name = name.trim().to_lowercase();
    let name = name.trim_start_matches('\u{b0}');
    let name = name
        .strip_prefix("degrees ")
        .or_else(|| name.strip_prefix("degree "))
        .filter(|rest| matches!(rest.trim(), "c" | "f" | "celsius" | "fahrenheit"))
        .unwrap_or(name)
        .trim();
    let name = name.trim_start_matches('\u{b0}');
    UNITS.iter().find(|u| u.names.contains(&name))
}

/// `value` in `from` converted to `to`; `None` across families.
pub fn convert_value(value: f64, from: &Unit, to: &Unit) -> Option<f64> {
    if from.dim != to.dim {
        return None;
    }
    let base = (value + from.offset) * from.factor;
    let out = base / to.factor - to.offset;
    out.is_finite().then_some(out)
}

/// `<n> <unit> in|to <unit>`: `5 km in miles`, `70F to C`, `-40 °C to °F`.
fn convert(q: &str) -> Option<Answer> {
    let lower = q.to_lowercase();
    let (number, rest) = split_number(&lower)?;
    let words: Vec<&str> = rest.split_whitespace().collect();
    // Every `in`/`to` is a candidate separator: `5 in in cm` splits at the second.
    let (from, to) = (1..words.len().saturating_sub(1))
        .filter(|&i| matches!(words[i], "in" | "to"))
        .find_map(|i| {
            Some((
                unit(&words[..i].join(" "))?,
                unit(&words[i + 1..].join(" "))?,
            ))
        })?;
    let value = convert_value(number, from, to)?;
    let from_text = format_number(number, MATH_DIGITS);
    let shown = format!("{} {}", format_number(value, UNIT_DIGITS), to.symbol);
    Some(Answer {
        kind: AnswerKind::Unit,
        title: format!("{from_text} {} = {shown}", from.symbol),
        copy: shown,
        value,
    })
}

/// A leading signed decimal number and what follows it.
fn split_number(s: &str) -> Option<(f64, &str)> {
    let s = s.trim_start();
    let digits_from = usize::from(s.starts_with(['-', '+']));
    let end = s[digits_from..]
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .map_or(s.len(), |i| i + digits_from);
    let text = &s[..end];
    if !text.bytes().any(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((text.parse().ok()?, &s[end..]))
}

// ------------------------------------------------------------- formatting

/// `value` to `digits` significant digits, trailing zeros trimmed: `87`, `1.41421356237`,
/// `0.3`; very large or small magnitudes in scientific form (`1.5e20`).
pub fn format_number(value: f64, digits: i32) -> String {
    if value == 0.0 {
        return "0".into();
    }
    if !value.is_finite() {
        return value.to_string();
    }
    let magnitude = value.abs().log10().floor() as i32;
    if !(-6..15).contains(&magnitude) {
        let s = format!("{:.*e}", (digits - 1).max(0) as usize, value);
        let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
        return format!("{}e{exp}", trim_zeros(mantissa));
    }
    let decimals = (digits - 1 - magnitude).clamp(0, 17) as usize;
    let s = trim_zeros(&format!("{value:.decimals$}")).to_owned();
    if s == "-0" { "0".into() } else { s }
}

fn trim_zeros(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calc(q: &str) -> Option<String> {
        answer(q)
            .filter(|a| a.kind == AnswerKind::Math)
            .map(|a| a.copy)
    }

    fn conv(q: &str) -> Option<String> {
        answer(q)
            .filter(|a| a.kind == AnswerKind::Unit)
            .map(|a| a.copy)
    }

    #[test]
    fn parses_and_respects_precedence() {
        let cases = [
            ("12*7+3", "87"),
            ("1 + 2 * 3", "7"),
            ("(1 + 2) * 3", "9"),
            ("2^3^2", "512"),
            ("-2^2", "-4"),
            ("2^-1", "0.5"),
            ("10 / 4", "2.5"),
            ("7 % 3", "1"),
            ("0.1 + 0.2", "0.3"),
            ("sqrt(2)", "1.41421356237"),
            ("sqrt 16 + 1", "5"),
            ("2pi", "6.28318530718"),
            ("3(4+1)", "15"),
            ("ln(e)", "1"),
            ("log(1000)", "3"),
            ("abs(-3)", "3"),
            ("round(2.5)", "3"),
            ("cos(0)", "1"),
            ("sin(0)", "0"),
            ("tan(0)", "0"),
            ("-(2)", "-2"),
            ("2 \u{d7} 3 \u{f7} 4", "1.5"),
            ("2e3 + 1", "2001"),
            ("(1 + 2", "3"),
        ];
        for (q, want) in cases {
            assert_eq!(calc(q).as_deref(), Some(want), "{q}");
        }
    }

    #[test]
    fn percent_of_and_percent_change() {
        assert_eq!(calc("15% of 80").as_deref(), Some("12"));
        assert_eq!(calc("50%").as_deref(), Some("0.5"));
        assert_eq!(calc("100 + 10%").as_deref(), Some("110"));
        assert_eq!(calc("200 - 25%").as_deref(), Some("150"));
        assert_eq!(calc("10 % 4").as_deref(), Some("2"));
    }

    #[test]
    fn division_by_zero_and_junk_give_no_row() {
        for q in [
            "1/0", "5 % 0", "1/(2-2)", "ln(0)", "sqrt(-1)", "2 +", "foo + 1", "1 ++", "()", "",
            "10^400",
        ] {
            assert_eq!(answer(q), None, "{q}");
        }
    }

    #[test]
    fn a_number_or_a_constant_alone_gives_no_row() {
        for q in [
            "42",
            "-5",
            "3.14",
            "pi",
            "e",
            "(4)",
            "hello world",
            "new boost",
        ] {
            assert_eq!(answer(q), None, "{q}");
        }
    }

    #[test]
    fn length() {
        assert_eq!(conv("5 km in miles").as_deref(), Some("3.10686 mi"));
        assert_eq!(conv("12 in to cm").as_deref(), Some("30.48 cm"));
        assert_eq!(conv("5 in in cm").as_deref(), Some("12.7 cm"));
        assert_eq!(conv("1 mile to feet").as_deref(), Some("5280 ft"));
    }

    #[test]
    fn mass() {
        assert_eq!(conv("1 kg to lb").as_deref(), Some("2.20462 lb"));
        assert_eq!(conv("16 oz in pounds").as_deref(), Some("1 lb"));
    }

    #[test]
    fn volume() {
        assert_eq!(conv("1 gallon to liters").as_deref(), Some("3.78541 l"));
        assert_eq!(conv("2 cups in fl oz").as_deref(), Some("16 fl oz"));
    }

    #[test]
    fn temperature() {
        assert_eq!(conv("70 F to C").as_deref(), Some("21.1111 \u{b0}C"));
        assert_eq!(conv("70F to C").as_deref(), Some("21.1111 \u{b0}C"));
        assert_eq!(
            conv("-40 \u{b0}C to \u{b0}F").as_deref(),
            Some("-40 \u{b0}F")
        );
        assert_eq!(conv("0 c in k").as_deref(), Some("273.15 K"));
        assert_eq!(
            conv("100 degrees celsius to f").as_deref(),
            Some("212 \u{b0}F")
        );
    }

    #[test]
    fn speed() {
        assert_eq!(conv("100 km/h to mph").as_deref(), Some("62.1371 mph"));
        assert_eq!(conv("10 m/s in km/h").as_deref(), Some("36 km/h"));
    }

    #[test]
    fn area() {
        assert_eq!(conv("1 acre to m2").as_deref(), Some("4046.86 m\u{b2}"));
        assert_eq!(
            conv("1 hectare in square meters").as_deref(),
            Some("10000 m\u{b2}")
        );
    }

    #[test]
    fn data_size() {
        assert_eq!(conv("1 GiB to MB").as_deref(), Some("1073.74 MB"));
        assert_eq!(conv("8 bits in bytes").as_deref(), Some("1 B"));
        assert_eq!(conv("100 mbit to MB").as_deref(), Some("12.5 MB"));
    }

    #[test]
    fn time() {
        assert_eq!(conv("90 min to hours").as_deref(), Some("1.5 h"));
        assert_eq!(conv("1 week in days").as_deref(), Some("7 d"));
        assert_eq!(conv("1 year to weeks").as_deref(), Some("52.1775 wk"));
    }

    #[test]
    fn unit_forms_that_do_not_convert() {
        for q in [
            "5 km in kg",
            "5 km",
            "km in mi",
            "5 km in",
            "5 parsecs in m",
            "5 usd to eur",
        ] {
            assert_eq!(answer(q), None, "{q}");
        }
    }

    #[test]
    fn unit_answer_text() {
        let a = answer("5 km in miles").unwrap();
        assert_eq!(a.title, "5 km = 3.10686 mi");
        let a = answer("12*7+3").unwrap();
        assert_eq!(a.title, "12*7+3 = 87");
        assert_eq!(a.copy, "87");
    }

    #[test]
    fn every_unit_name_is_lower_case_and_unique() {
        let mut seen: Vec<&str> = Vec::new();
        for u in UNITS {
            assert_eq!(unit(u.symbol), Some(u), "{}", u.symbol);
            for &n in u.names {
                assert_eq!(n, n.to_lowercase(), "{n}");
                assert!(!seen.contains(&n), "{n} twice");
                seen.push(n);
            }
        }
    }

    #[test]
    fn formatting_trims_zeros() {
        assert_eq!(format_number(87.0, 12), "87");
        assert_eq!(format_number(2.5, 12), "2.5");
        assert_eq!(format_number(-0.000_000_1, 12), "-1e-7");
        assert_eq!(format_number(1.5e20, 12), "1.5e20");
        assert_eq!(format_number(1.0 / 3.0, 6), "0.333333");
        assert_eq!(format_number(-0.0, 12), "0");
        assert_eq!(format_number(1e-13, 6), "1e-13");
    }
}
