//! FELanguage formatting natives the menu movies call (doc 31, Milestone 6). Retail registrar 825AA5E8 (TU3);
//! research `.local/research/felanguage.md`.
//!
//! Placeholders in the language strings are `{N:type[.P]}` (N one digit, P one digit of precision, default 3);
//! `{{` stays literal. Number separators are data (`NumberFormat`), retail's table 0x82FDE250 holds `. , 3`.
//!
//! NOT RETAIL YET: APT values are int or float at runtime, ours are all f64; an integral number is treated as an
//! int (grouped), anything else as a float. Per-language separators and the typed formats other than `d` / `f`
//! (points, distance, time, ...) are not decoded; those placeholders take the argument text as is.
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct NumberFormat {
    pub decimal: char,
    pub group: char,
    pub group_size: usize,
}

impl Default for NumberFormat {
    fn default() -> Self {
        Self { decimal: '.', group: ',', group_size: 3 }
    }
}

impl NumberFormat {
    pub fn int(&self, n: i64) -> String {
        let digits = n.unsigned_abs().to_string();
        let mut out = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && self.group_size > 0 && (digits.len() - i) % self.group_size == 0 {
                out.push(self.group);
            }
            out.push(c);
        }
        if n < 0 {
            out.insert(0, '-');
        }
        out
    }
    /// Exactly `decimals` digits, rounded half up with carry (retail: 9.999 at 2 gives 10.00).
    pub fn float(&self, x: f64, decimals: usize) -> String {
        if !x.is_finite() {
            return String::new();
        }
        let scale = 10f64.powi(decimals.min(9) as i32);
        let scaled = (x.abs() * scale + 0.5).floor();
        let whole = (scaled / scale).floor();
        let frac = scaled - whole * scale;
        let mut out = String::new();
        if x < 0.0 && scaled > 0.0 {
            out.push('-');
        }
        out.push_str(&(whole as u64).to_string());
        if decimals > 0 {
            out.push(self.decimal);
            out.push_str(&format!("{:0width$}", frac as u64, width = decimals.min(9)));
        }
        out
    }
}

/// A script value as the natives see it.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Number(f64),
    Bool(bool),
    Text(String),
    Undefined,
}

fn integral(x: f64) -> bool {
    x.is_finite() && x.fract() == 0.0 && x.abs() < 9.0e15
}

/// GetLocalizedFormatNumberString (825AAB50): int grouped, float with `decimals` (default 3); other values give
/// an empty string (retail returns an uninitialised buffer).
pub fn number_string(format: &NumberFormat, value: &Arg, decimals: Option<usize>) -> String {
    match value {
        Arg::Number(x) if integral(*x) => format.int(*x as i64),
        Arg::Number(x) => format.float(*x, decimals.unwrap_or(3)),
        _ => String::new(),
    }
}

/// One placeholder `{N:type[.P]}` at `chars[i]`; returns (index, type, precision, length).
fn placeholder(chars: &[char], i: usize) -> Option<(usize, String, Option<usize>, usize)> {
    let close = chars[i..].iter().position(|c| *c == '}')? + i;
    let body: String = chars[i + 1..close].iter().collect();
    let (n, kind) = body.split_once(':').unwrap_or((&body, ""));
    let index = n.parse::<usize>().ok().filter(|n| *n <= 9 && n.to_string().len() == 1)?;
    let (kind, precision) = match kind.split_once('.') {
        Some((k, p)) => (k.to_string(), p.parse().ok()),
        None => (kind.to_string(), None),
    };
    Some((index, kind, precision, close - i + 1))
}

/// GetLocalizedFormatString (825AA8B8): every argument is turned into text first (int grouped, bool 0 / 1, float
/// with 2 decimals, text as is), then each placeholder is replaced by its argument's text; the type and precision
/// in the string are ignored. At most 6 arguments.
pub fn format_string(format: &NumberFormat, template: &str, args: &[Arg]) -> String {
    let texts: Vec<String> = args
        .iter()
        .take(6)
        .map(|a| match a {
            Arg::Number(x) if integral(*x) => format.int(*x as i64),
            Arg::Number(x) => format.float(*x, 2),
            Arg::Bool(b) => (*b as i32).to_string(),
            Arg::Text(t) => t.clone(),
            Arg::Undefined => String::new(),
        })
        .collect();
    substitute(template, |index, _, _| texts.get(index).cloned(), false)
}

/// GetLocalizedFormatStringCustom (825AAAA8): the arguments come as one comma list (up to 20, 63 characters each);
/// `d` placeholders format an integer (grouped), `f` a float with the placeholder's precision (default 3), other
/// types the text as is. Retail leaves a placeholder literal when its index is not above the highest one used.
pub fn format_string_custom(format: &NumberFormat, template: &str, list: &str) -> String {
    let args: Vec<String> = list.split(',').take(20).map(|a| a.chars().take(63).collect()).collect();
    substitute(
        template,
        |index, kind, precision| {
            let a = args.get(index)?;
            Some(match kind {
                "d" => format.int(c_atoi(a)),
                "f" => format.float(a.trim().parse::<f64>().unwrap_or(0.0), precision.unwrap_or(3)),
                _ => a.clone(),
            })
        },
        true,
    )
}

fn c_atoi(s: &str) -> i64 {
    let s = s.trim_start();
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let n: i64 = digits.chars().take_while(char::is_ascii_digit).fold(0i64, |n, c| {
        n.saturating_mul(10).saturating_add(c as i64 - '0' as i64)
    });
    if neg { -n } else { n }
}

fn substitute(
    template: &str,
    value: impl Fn(usize, &str, Option<usize>) -> Option<String>,
    increasing: bool,
) -> String {
    let chars: Vec<char> = template.chars().collect();
    let mut out = String::new();
    let mut highest: Option<usize> = None;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            if chars.get(i + 1) == Some(&'{') {
                out.push('{');
                i += 2;
                continue;
            }
            if let Some((index, kind, precision, len)) = placeholder(&chars, i) {
                let allowed = !increasing || highest.is_none_or(|h| index > h);
                if allowed {
                    if let Some(v) = value(index, &kind, precision) {
                        out.push_str(&v);
                        highest = Some(index);
                        i += len;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Resolves a label id (or `#literal`) to its template; unknown ids give the id itself (as `apt_text::localize`).
pub fn template<'a>(language: &'a BTreeMap<String, String>, id: &'a str) -> &'a str {
    if let Some(literal) = id.strip_prefix('#') {
        return literal;
    }
    language.get(id).map_or(id, String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fe_language_numbers_follow_retail() {
        let f = NumberFormat::default();
        assert_eq!(number_string(&f, &Arg::Number(25000.0), None), "25,000");
        assert_eq!(number_string(&f, &Arg::Number(-1234567.0), None), "-1,234,567");
        assert_eq!(number_string(&f, &Arg::Number(1.5), Some(2)), "1.50");
        assert_eq!(f.float(9.999, 2), "10.00");
        assert_eq!(number_string(&f, &Arg::Undefined, None), "");
    }

    #[test]
    fn fe_language_formats_follow_retail() {
        let f = NumberFormat::default();
        assert_eq!(format_string(&f, "{0:d}%", &[Arg::Number(50.0)]), "50%");
        assert_eq!(format_string(&f, "{0:d}%", &[Arg::Number(50.5)]), "50.50%");
        assert_eq!(format_string(&f, "{{0}} {0:s}", &[Arg::Text("x".into())]), "{0}} x");
        let milestone = "Board Sales Milestone {0:d}/{1:d}";
        assert_eq!(format_string_custom(&f, milestone, "2,6"), "Board Sales Milestone 2/6");
        assert_eq!(format_string_custom(&f, "{0:f.1} {1:d}", "1.25,12000"), "1.3 12,000");
        // Indices must increase: a repeated index stays literal.
        assert_eq!(format_string_custom(&f, "{0:d} {0:d}", "1"), "1 {0:d}");
    }
}
