use ratatui::style::Color;

/// Format a byte count as a short human-readable string (e.g. `12.3G`).
pub fn fmt_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes}B")
    } else if value >= 100.0 {
        format!("{value:.0}{}", UNITS[unit])
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

/// Format a bytes-per-second rate (e.g. `1.2M/s`).
pub fn fmt_rate(bytes_per_sec: u64) -> String {
    format!("{}/s", fmt_bytes(bytes_per_sec))
}

/// Format seconds as `hh:mm:ss`, or `Nd hh:mm` once it exceeds a day.
pub fn fmt_duration(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;
    let s = secs % 60;
    if days > 0 {
        format!("{days}d {hours:02}:{mins:02}")
    } else {
        format!("{hours:02}:{mins:02}:{s:02}")
    }
}

/// One-letter process state in the htop convention.
pub fn status_code(status: &str) -> char {
    match status {
        s if s.starts_with("Run") => 'R',
        s if s.starts_with("Sleep") => 'S',
        s if s.starts_with("Idle") => 'I',
        s if s.starts_with("Stop") => 'T',
        s if s.starts_with("Zombie") => 'Z',
        s if s.starts_with("Uninterruptible") => 'D',
        s if s.starts_with("Dead") => 'X',
        s if s.starts_with("Tracing") => 't',
        s if s.starts_with("Parked") => 'P',
        s if s.starts_with("Wak") => 'W',
        s if s.starts_with("Lock") => 'L',
        _ => '?',
    }
}

/// Map a 0..=100 percentage onto a green → yellow → red gradient.
pub fn heat(pct: f64) -> Color {
    let p = pct.clamp(0.0, 100.0) / 100.0;
    let (r, g, b) = if p < 0.5 {
        let t = p / 0.5;
        lerp3((60.0, 220.0, 140.0), (240.0, 200.0, 40.0), t)
    } else {
        let t = (p - 0.5) / 0.5;
        lerp3((240.0, 200.0, 40.0), (250.0, 70.0, 70.0), t)
    };
    Color::Rgb(r as u8, g as u8, b as u8)
}

fn lerp3(a: (f64, f64, f64), b: (f64, f64, f64), t: f64) -> (f64, f64, f64) {
    (
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
    )
}

/// Render a horizontal bar of `width` cells for a 0..=100 percentage using
/// eighth-block characters for sub-cell resolution.
pub fn bar(pct: f64, width: usize) -> String {
    const BLOCKS: [char; 8] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
    let filled = (pct.clamp(0.0, 100.0) / 100.0) * width as f64;
    let full = filled.floor() as usize;
    let rem = filled - full as f64;
    let mut s = String::with_capacity(width * 3);
    for _ in 0..full.min(width) {
        s.push('█');
    }
    if full < width {
        let idx = (rem * 8.0).round() as usize;
        if idx > 0 {
            s.push(BLOCKS[idx.min(8) - 1]);
        } else {
            s.push(' ');
        }
        for _ in (full + 1)..width {
            s.push(' ');
        }
    }
    s
}

/// Truncate a string to at most `max` characters, appending `…` if cut.
pub fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}
