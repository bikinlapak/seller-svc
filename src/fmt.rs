//! Display formatting. The backend formats money and quantities so every
//! screen says "Rp60,000" the same way (contract: `Idr` is whole rupiah).

use crate::contract::Idr;

/// "Rp1,250,000".
pub fn idr(v: Idr) -> String {
    let neg = v < 0;
    let digits = v.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    format!("{}Rp{out}", if neg { "-" } else { "" })
}

/// "Rp167k", "Rp6.8M" — for tight spots like card headers and tiles.
pub fn idr_short(v: Idr) -> String {
    let a = v.abs() as f64;
    let sign = if v < 0 { "-" } else { "" };
    if a >= 1_000_000.0 {
        format!("{sign}Rp{}M", trim(a / 1_000_000.0, 1))
    } else if a >= 1_000.0 {
        format!("{sign}Rp{}k", (a / 1_000.0).round() as i64)
    } else {
        format!("{sign}Rp{}", a as i64)
    }
}

/// One decimal, without a trailing ".0": 1.6, 2, 12.5.
pub fn trim(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

/// "4 kg", "14 boxes", "1 box".
pub fn qty(n: i64, unit: &str) -> String {
    format!("{n} {}", plural(unit, n))
}

pub fn plural(unit: &str, n: i64) -> String {
    if n == 1 || matches!(unit, "kg" | "L" | "pcs") {
        return unit.to_owned();
    }
    match unit {
        "box" => "boxes".into(),
        "loaf" => "loaves".into(),
        u => format!("{u}s"),
    }
}

/// "3.2 kg" from grams.
pub fn kg(grams: i64) -> String {
    format!("{} kg", trim(grams as f64 / 1000.0, 1))
}

pub fn percent(ratio: f64) -> i64 {
    (ratio * 100.0).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rupiah() {
        assert_eq!(idr(60_000), "Rp60,000");
        assert_eq!(idr(1_250_000), "Rp1,250,000");
        assert_eq!(idr(500), "Rp500");
        assert_eq!(idr_short(167_400), "Rp167k");
        assert_eq!(idr_short(6_830_000), "Rp6.8M");
    }

    #[test]
    fn quantities() {
        assert_eq!(qty(14, "box"), "14 boxes");
        assert_eq!(qty(1, "box"), "1 box");
        assert_eq!(qty(4, "kg"), "4 kg");
        assert_eq!(trim(2.0, 1), "2");
        assert_eq!(trim(1.64, 1), "1.6");
        assert_eq!(kg(3_240), "3.2 kg");
    }
}
