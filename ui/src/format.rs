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
        if i > 0 && (s.len() - i) % 3 == 0 {
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
pub fn clock(ts: f64) -> String {
    let d = js_sys::Date::new(&ts.into());
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        d.get_hours(),
        d.get_minutes(),
        d.get_seconds(),
        d.get_milliseconds()
    )
}

/// `KeyboardEvent.key`, or "" when the event has none. Chromium fires bare
/// "keydown" events with no `key` (e.g. picking an autofill suggestion), and
/// web-sys's `key()` panics on those.
pub fn key_of(e: &web_sys::KeyboardEvent) -> String {
    js_sys::Reflect::get(e, &"key".into())
        .ok()
        .and_then(|k| k.as_string())
        .unwrap_or_default()
}
