//! Number formatting that matches what gethomepage shows (en locale), so the
//! two dashboards can be compared digit for digit.

const BYTE_UNITS: [&str; 9] = ["B", "kB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];
const BIBYTE_UNITS: [&str; 9] = ["B", "kiB", "MiB", "GiB", "TiB", "PiB", "EiB", "ZiB", "YiB"];

fn group_thousands(int_part: &str) -> String {
    let mut out = String::with_capacity(int_part.len() + int_part.len() / 3);
    let len = int_part.len();
    for (i, c) in int_part.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn round_to(value: f64, decimals: usize) -> f64 {
    let p = 10f64.powi(decimals as i32);
    (value * p).round() / p
}

/// `Intl.NumberFormat("en", { minimumFractionDigits, maximumFractionDigits })`.
fn fixed(value: f64, min_frac: usize, max_frac: usize) -> String {
    if !value.is_finite() {
        return "-".into();
    }
    let rounded = round_to(value.abs(), max_frac);
    let mut s = format!("{rounded:.max_frac$}");
    if let Some(dot) = s.find('.') {
        let keep = dot + 1 + min_frac;
        while s.len() > keep && s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    let (int_part, frac) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s.as_str(), None),
    };
    let mut out = String::new();
    if value < 0.0 && rounded != 0.0 {
        out.push('-');
    }
    out.push_str(&group_thousands(int_part));
    if let Some(f) = frac {
        out.push('.');
        out.push_str(f);
    }
    out
}

/// `common.number` with the default three fraction digits.
pub fn number(value: f64) -> String {
    fixed(value, 0, 3)
}

/// `common.number` with an explicit `maximumFractionDigits`.
pub fn number_max(value: f64, max_frac: usize) -> String {
    fixed(value, 0, max_frac)
}

pub fn int(value: i64) -> String {
    fixed(value as f64, 0, 0)
}

/// `common.percent`: the value is already a percentage (19.3 -> "19%").
pub fn percent(value: f64) -> String {
    format!("{}%", fixed(value, 0, 0))
}

/// JavaScript `Number(value.toPrecision(3))`.
fn precision3(value: f64) -> f64 {
    if value == 0.0 {
        return 0.0;
    }
    let digits = value.abs().log10().floor() as i32 + 1;
    let p = 10f64.powi(3 - digits);
    (value * p).round() / p
}

/// `common.bytes` / `common.bbytes` (pretty-bytes). `max_frac = None` keeps
/// three significant digits, which is the pretty-bytes default.
pub fn bytes(value: f64, binary: bool, max_frac: Option<usize>) -> String {
    let units = if binary { &BIBYTE_UNITS } else { &BYTE_UNITS };
    if !value.is_finite() {
        return "-".into();
    }
    let (sign, mut n) = if value < 0.0 {
        ("-", -value)
    } else {
        ("", value)
    };
    if n < 1.0 {
        return format!("{sign}{} {}", fixed(n, 0, max_frac.unwrap_or(3)), units[0]);
    }
    let base: f64 = if binary { 1024.0 } else { 1000.0 };
    let exponent = ((n.ln() / base.ln()).floor() as usize).min(units.len() - 1);
    n /= base.powi(exponent as i32);
    let text = match max_frac {
        Some(m) => fixed(n, 0, m),
        None => fixed(precision3(n), 0, 3),
    };
    format!("{sign}{text} {}", units[exponent])
}

/// `common.byterate`: bytes per second with a fixed number of decimals.
pub fn byterate(value: f64, decimals: usize) -> String {
    if !value.is_finite() || value <= 0.0 {
        return "0 B/s".into();
    }
    let i = ((value.ln() / 1000f64.ln()).floor().max(0.0) as usize).min(BYTE_UNITS.len() - 1);
    let scaled = value / 1000f64.powi(i as i32);
    format!("{} {}/s", fixed(scaled, decimals, decimals), BYTE_UNITS[i])
}

/// `common.duration`: the two most significant units, e.g. "6d15h".
pub fn duration(seconds: u64) -> String {
    let mo = seconds / (3600 * 24 * 31);
    let d = (seconds % (3600 * 24 * 31)) / (3600 * 24);
    let h = (seconds % (3600 * 24)) / 3600;
    let m = (seconds % 3600) / 60;
    let s = seconds % 60;
    let mut out = String::new();
    if mo > 0 {
        out.push_str(&format!("{mo}mo"));
    }
    if d > 0 {
        out.push_str(&format!("{d}d"));
    }
    if h > 0 && mo == 0 {
        out.push_str(&format!("{h}h"));
    }
    if m > 0 && mo == 0 && d == 0 {
        out.push_str(&format!("{m}m"));
    }
    if s > 0 && mo == 0 && d == 0 && h == 0 {
        out.push_str(&format!("{s}s"));
    }
    out
}

/// Parses a Kubernetes resource quantity ("250m", "12345678n", "16Gi", "2").
pub fn quantity(q: &str) -> Option<f64> {
    let q = q.trim();
    let bytes = q.as_bytes();
    let mut end = 0;
    while end < bytes.len() {
        let c = bytes[end];
        let numeric = c.is_ascii_digit() || c == b'.' || (end == 0 && (c == b'-' || c == b'+'));
        let exponent = (c == b'e' || c == b'E')
            && end > 0
            && bytes
                .get(end + 1)
                .is_some_and(|n| n.is_ascii_digit() || *n == b'-' || *n == b'+');
        if !(numeric || exponent) {
            break;
        }
        end += if exponent { 2 } else { 1 };
    }
    let value: f64 = q[..end.min(q.len())].parse().ok()?;
    let factor = match &q[end.min(q.len())..] {
        "" => 1.0,
        "n" => 1e-9,
        "u" => 1e-6,
        "m" => 1e-3,
        "k" => 1e3,
        "M" => 1e6,
        "G" => 1e9,
        "T" => 1e12,
        "P" => 1e15,
        "E" => 1e18,
        "Ki" => 1024.0,
        "Mi" => 1024f64.powi(2),
        "Gi" => 1024f64.powi(3),
        "Ti" => 1024f64.powi(4),
        "Pi" => 1024f64.powi(5),
        "Ei" => 1024f64.powi(6),
        _ => return None,
    };
    Some(value * factor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_group_and_trim() {
        assert_eq!(number(75831.0), "75,831");
        assert_eq!(number(0.0), "0");
        assert_eq!(number_max(0.03471, 4), "0.0347");
        assert_eq!(number_max(0.001, 4), "0.001");
        assert_eq!(number_max(4.25, 1), "4.3");
        assert_eq!(number_max(-0.01, 1), "0");
        assert_eq!(int(186056), "186,056");
    }

    #[test]
    fn percent_rounds() {
        assert_eq!(percent(19.3), "19%");
        assert_eq!(percent(57.6), "58%");
        assert_eq!(percent(0.0), "0%");
    }

    #[test]
    fn bytes_match_pretty_bytes() {
        assert_eq!(bytes(227_000_000.0, false, None), "227 MB");
        assert_eq!(bytes(1_930_000_000.0, false, None), "1.93 GB");
        assert_eq!(bytes(47_100_000.0, false, None), "47.1 MB");
        assert_eq!(bytes(8_800_000_000.0, false, None), "8.8 GB");
        assert_eq!(bytes(0.0, false, None), "0 B");
        assert_eq!(bytes(82_141_245_440.0, true, Some(1)), "76.5 GiB");
        assert_eq!(bytes(53_687_091_200.0, true, Some(0)), "50 GiB");
    }

    #[test]
    fn rates() {
        assert_eq!(byterate(143_800_000.0, 1), "143.8 MB/s");
        assert_eq!(byterate(10_800_000.0, 1), "10.8 MB/s");
        assert_eq!(byterate(0.0, 0), "0 B/s");
        assert_eq!(byterate(1_500.0, 0), "2 kB/s");
    }

    #[test]
    fn durations() {
        assert_eq!(duration(575_290), "6d15h");
        assert_eq!(duration(59), "59s");
        assert_eq!(duration(3_700), "1h1m");
        assert_eq!(duration(40 * 86_400), "1mo9d");
    }

    #[test]
    fn quantities() {
        assert_eq!(quantity("250m"), Some(0.25));
        assert_eq!(quantity("2"), Some(2.0));
        assert_eq!(quantity("16Gi"), Some(16.0 * 1024.0 * 1024.0 * 1024.0));
        assert_eq!(quantity("65806532Ki"), Some(65_806_532.0 * 1024.0));
        assert!((quantity("123456789n").unwrap() - 0.123456789).abs() < 1e-12);
        assert_eq!(quantity("1e3"), Some(1000.0));
        assert_eq!(quantity("5E"), Some(5e18));
        assert_eq!(quantity("bogus"), None);
    }
}
