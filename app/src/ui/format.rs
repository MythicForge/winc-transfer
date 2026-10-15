pub fn bytes(n: f64) -> String {
    if n < 1000.0 {
        return format!("{} B", n.round());
    }
    const U: [&str; 4] = ["KB", "MB", "GB", "TB"];
    let mut v = n / 1000.0;
    let mut i = 0;
    while v >= 1000.0 && i < U.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if v < 10.0 {
        format!("{v:.1} {}", U[i])
    } else {
        format!("{v:.0} {}", U[i])
    }
}

pub fn rate(bps: f64) -> String {
    format!("{}/s", bytes(bps))
}

/// 12840 → "12,840"
pub fn count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn eta(bytes_left: f64, bps: f64) -> String {
    if bps <= 0.0 {
        return "—".into();
    }
    let s = bytes_left / bps;
    if s < 60.0 {
        return format!("{}s", s.ceil());
    }
    if s < 3600.0 {
        return format!("{}m {}s", (s / 60.0).floor(), (s % 60.0).round());
    }
    format!("{}h {}m", (s / 3600.0).floor(), ((s % 3600.0) / 60.0).round())
}

/// hh:mm:ss.mmm in local time, for log rows.
pub fn clock(ts_ms: u64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(ts_ms as i64) {
        chrono::LocalResult::Single(t) => t.format("%H:%M:%S%.3f").to_string(),
        _ => "--:--:--.---".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(bytes(999.0), "999 B");
        assert_eq!(bytes(1_500_000.0), "1.5 MB");
        assert_eq!(bytes(250_000_000_000.0), "250 GB");
        assert_eq!(count(12840), "12,840");
        assert_eq!(count(7), "7");
        assert_eq!(eta(0.0, 0.0), "—");
        assert_eq!(eta(90.0, 1.0), "1m 30s");
    }
}
