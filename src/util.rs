use ratatui::style::Color;

/// Format a byte count with decimal units and full suffixes (e.g. `12.3GB`).
pub fn fmt_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
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

/// Format a bytes-per-second rate as bits per second with decimal units,
/// following networking convention (e.g. `1.2Mbps`).
pub fn fmt_rate(bytes_per_sec: u64) -> String {
    const UNITS: [&str; 6] = ["bps", "Kbps", "Mbps", "Gbps", "Tbps", "Pbps"];
    let bits = bytes_per_sec.saturating_mul(8);
    let mut value = bits as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bits}{}", UNITS[unit])
    } else if value >= 100.0 {
        format!("{value:.0}{}", UNITS[unit])
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_bytes_uses_thousands_and_full_suffixes() {
        assert_eq!(fmt_bytes(0), "0B");
        assert_eq!(fmt_bytes(999), "999B");
        assert_eq!(fmt_bytes(1000), "1.0KB");
        assert_eq!(fmt_bytes(1536), "1.5KB");
        assert_eq!(fmt_bytes(100_000), "100KB");
        assert_eq!(fmt_bytes(12_300_000_000), "12.3GB");
        assert_eq!(fmt_bytes(994_000_000_000), "994GB");
        assert_eq!(fmt_bytes(2_000_000_000_000), "2.0TB");
        assert_eq!(fmt_bytes(u64::MAX), "18447PB");
    }

    #[test]
    fn fmt_rate_converts_bytes_to_decimal_bits() {
        assert_eq!(fmt_rate(0), "0bps");
        assert_eq!(fmt_rate(100), "800bps");
        assert_eq!(fmt_rate(125), "1.0Kbps");
        assert_eq!(fmt_rate(2048), "16.4Kbps");
        assert_eq!(fmt_rate(12_500), "100Kbps");
        assert_eq!(fmt_rate(125_000_000), "1.0Gbps");
        assert_eq!(fmt_rate(119_209_289), "954Mbps");
        assert_eq!(fmt_rate(1_250_000_000_000), "10.0Tbps");
        assert_eq!(fmt_rate(u64::MAX), "18447Pbps");
    }

    #[test]
    fn fmt_duration_switches_to_days() {
        assert_eq!(fmt_duration(0), "00:00:00");
        assert_eq!(fmt_duration(3661), "01:01:01");
        assert_eq!(fmt_duration(86_399), "23:59:59");
        assert_eq!(fmt_duration(86_400), "1d 00:00");
        assert_eq!(
            fmt_duration(2 * 86_400 + 3 * 3600 + 4 * 60 + 59),
            "2d 03:04"
        );
    }

    #[test]
    fn status_code_maps_known_states() {
        assert_eq!(status_code("Runnable"), 'R');
        assert_eq!(status_code("Sleeping"), 'S');
        assert_eq!(status_code("Idle"), 'I');
        assert_eq!(status_code("Stopped"), 'T');
        assert_eq!(status_code("Zombie"), 'Z');
        assert_eq!(status_code("Uninterruptible Disk Sleep"), 'D');
        assert_eq!(status_code("Dead"), 'X');
        assert_eq!(status_code("Tracing"), 't');
        assert_eq!(status_code("Parked"), 'P');
        assert_eq!(status_code("Waking"), 'W');
        assert_eq!(status_code("LockBlocked"), 'L');
        assert_eq!(status_code("Unknown"), '?');
        assert_eq!(status_code(""), '?');
    }

    #[test]
    fn heat_gradient_endpoints_and_clamping() {
        assert_eq!(heat(0.0), Color::Rgb(60, 220, 140));
        assert_eq!(heat(50.0), Color::Rgb(240, 200, 40));
        assert_eq!(heat(100.0), Color::Rgb(250, 70, 70));
        assert_eq!(heat(-10.0), heat(0.0));
        assert_eq!(heat(500.0), heat(100.0));
    }

    #[test]
    fn heat_gets_redder_as_pct_rises() {
        let Color::Rgb(_, g_low, _) = heat(10.0) else {
            panic!()
        };
        let Color::Rgb(_, g_high, _) = heat(90.0) else {
            panic!()
        };
        assert!(g_low > g_high);
    }

    #[test]
    fn bar_fills_full_cells() {
        assert_eq!(bar(0.0, 4), "    ");
        assert_eq!(bar(50.0, 4), "██  ");
        assert_eq!(bar(100.0, 4), "████");
        assert_eq!(bar(100.0, 0), "");
    }

    #[test]
    fn bar_uses_partial_block_for_remainder() {
        // 12.5% of 4 cells = half a cell → '▌'.
        assert_eq!(bar(12.5, 4), "▌   ");
        // A tiny remainder rounds down to a blank cell.
        assert_eq!(bar(1.0, 4), "    ");
    }

    #[test]
    fn bar_always_has_width_chars() {
        for pct in [0.0, 3.3, 33.3, 66.6, 99.9, 100.0, 150.0] {
            for w in 0..6 {
                assert_eq!(bar(pct, w).chars().count(), w, "pct={pct} w={w}");
            }
        }
    }

    #[test]
    fn ellipsize_truncates_by_chars() {
        assert_eq!(ellipsize("hello", 10), "hello");
        assert_eq!(ellipsize("hello", 5), "hello");
        assert_eq!(ellipsize("hello", 4), "hel…");
        assert_eq!(ellipsize("hello", 1), "…");
        assert_eq!(ellipsize("hello", 0), "");
        assert_eq!(ellipsize("héllo wörld", 6), "héllo…");
    }
}
