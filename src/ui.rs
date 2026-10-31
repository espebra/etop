use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::symbols::{self, Marker};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Axis, Block, BorderType, Cell, Chart, Clear, Dataset, Gauge, GraphType, LineGauge, Paragraph,
    Row, Sparkline, Table, Wrap,
};

use crate::app::{App, Mode, SortBy};
use crate::util::{bar, ellipsize, fmt_bytes, fmt_duration, fmt_rate, heat, status_code};

const ACCENT: Color = Color::Rgb(120, 170, 255);
const DIM: Color = Color::Rgb(110, 115, 130);
const RX: Color = Color::Rgb(80, 200, 255);
const TX: Color = Color::Rgb(255, 120, 200);
const MEM: Color = Color::Rgb(170, 130, 255);
const SWAP: Color = Color::Rgb(255, 170, 90);
const BORDER: Color = Color::Rgb(70, 75, 95);

fn block(title: Line<'_>) -> Block<'_> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(BORDER))
        .title(title)
}

fn title<'a>(name: &'a str, detail: impl Into<Span<'a>>) -> Line<'a> {
    let detail: Span<'a> = detail.into();
    let mut spans = vec![" ".into(), name.fg(ACCENT).bold(), " ".into()];
    if !detail.content.is_empty() {
        spans.push(detail);
        spans.push(" ".into());
    }
    Line::from(spans)
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let top_h = (area.height as f32 * 0.40).round().clamp(10.0, 22.0) as u16;
    let [header, top, procs, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(top_h),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(frame, app, header);

    let [cpu, mem, right] = Layout::horizontal([
        Constraint::Percentage(46),
        Constraint::Percentage(24),
        Constraint::Percentage(30),
    ])
    .areas(top);
    let [net, disk] =
        Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(right);

    draw_cpu(frame, app, cpu);
    draw_mem(frame, app, mem);
    draw_net(frame, app, net);
    draw_disks(frame, app, disk);
    draw_procs(frame, app, procs);
    draw_footer(frame, app, footer);

    match &app.mode {
        Mode::Help => draw_help(frame, area),
        Mode::ConfirmKill { pid, name, force } => draw_confirm(frame, area, *pid, name, *force),
        _ => {}
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let s = &app.snap;
    let info = &app.info;
    let cores = if info.physical_cores > 0 && info.physical_cores != info.logical_cores {
        format!("{}c/{}t", info.physical_cores, info.logical_cores)
    } else {
        format!("{}c", info.logical_cores)
    };
    let mut spans = vec![
        " etop ".bold().fg(Color::Black).bg(ACCENT),
        " ".into(),
        info.hostname.as_str().bold(),
        "  ".into(),
        info.os.as_str().fg(DIM),
        "  ".into(),
        format!("{} {}", info.cpu_brand, info.arch).fg(DIM),
        " ".into(),
        format!("({cores})").fg(DIM),
        "  up ".fg(DIM),
        fmt_duration(s.uptime).into(),
    ];
    if app.paused {
        spans.push("  ".into());
        spans.push(" PAUSED ".bold().fg(Color::Black).bg(Color::Yellow));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_cpu(frame: &mut Frame, app: &App, area: Rect) {
    let s = &app.snap;
    let pct = s.cpu_total as f64;
    let freq = if s.cpu_freq_mhz > 0 {
        format!(" · {:.2} GHz", s.cpu_freq_mhz as f64 / 1000.0)
    } else {
        String::new()
    };
    let detail = Line::from(vec![
        format!("{pct:5.1}%").fg(heat(pct)).bold(),
        freq.fg(DIM),
        format!(" · load {:.2} {:.2} {:.2}", s.load.0, s.load.1, s.load.2).fg(DIM),
    ]);
    let mut t = title("CPU", "");
    t.spans.truncate(3);
    t.spans.extend(detail.spans);
    t.spans.push(" ".into());
    let b = block(t);
    let inner = b.inner(area);
    frame.render_widget(b, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Per-core column layout: as many columns as needed to fit all cores.
    let n = s.cpu_cores.len().max(1);
    let rows = inner.height as usize;
    let cols = n.div_ceil(rows).max(1);
    let avail = inner.width.saturating_sub(20);
    // Wide format "nn ████████▏ 100%" or compact "nn ████▏100%" when crowded.
    let (col_w, bar_w, compact) = if 17 * cols as u16 <= avail {
        (17u16, 9usize, false)
    } else {
        (12u16, 5usize, true)
    };
    let cores_w = (col_w * cols as u16).min(avail);

    let [chart_area, cores_area] =
        Layout::horizontal([Constraint::Min(16), Constraint::Length(cores_w)]).areas(inner);

    // History chart, right-aligned so the newest sample touches the right edge.
    let width = (chart_area.width as usize * 2).max(2);
    let hist = &app.cpu_hist;
    let start = hist.len().saturating_sub(width);
    let offset = width.saturating_sub(hist.len());
    let points: Vec<(f64, f64)> = hist
        .iter()
        .skip(start)
        .enumerate()
        .map(|(i, v)| ((offset + i) as f64, *v))
        .collect();
    let ds = Dataset::default()
        .marker(Marker::Braille)
        .graph_type(GraphType::Area)
        .style(Style::new().fg(heat(pct)))
        .data(&points);
    let chart = Chart::new(vec![ds])
        .x_axis(Axis::default().bounds([0.0, width as f64]))
        .y_axis(
            Axis::default()
                .bounds([0.0, 100.0])
                .labels(["0", "50", "100"])
                .style(Style::new().fg(DIM)),
        )
        .legend_position(None);
    frame.render_widget(chart, chart_area);

    // Per-core bars.
    let per_col = n.div_ceil(cols);
    let col_rects = Layout::horizontal(vec![Constraint::Length(col_w); cols]).split(cores_area);
    for (c, rect) in col_rects.iter().enumerate() {
        let lines: Vec<Line> = s
            .cpu_cores
            .iter()
            .enumerate()
            .skip(c * per_col)
            .take(per_col)
            .map(|(i, v)| {
                let v = *v as f64;
                let pct = if compact {
                    format!("{v:>3.0}%")
                } else {
                    format!("{v:>4.0}%")
                };
                Line::from(vec![
                    format!("{i:>2} ").fg(DIM),
                    bar(v, bar_w).fg(heat(v)),
                    pct.fg(heat(v)),
                ])
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), *rect);
    }
}

fn draw_mem(frame: &mut Frame, app: &App, area: Rect) {
    let s = &app.snap;
    let total = s.mem_total.max(1);
    let used_pct = s.mem_used as f64 / total as f64;
    let b = block(title(
        "Memory",
        format!("{:.1}%", used_pct * 100.0)
            .fg(heat(used_pct * 100.0))
            .bold(),
    ));
    let inner = b.inner(area);
    frame.render_widget(b, area);
    if inner.height < 2 {
        return;
    }

    let [ram_l, ram_g, swap_l, swap_g, spark] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            "RAM ".fg(MEM).bold(),
            format!("{} / {}", fmt_bytes(s.mem_used), fmt_bytes(total)).into(),
            format!("  avail {}", fmt_bytes(s.mem_available)).fg(DIM),
        ])),
        ram_l,
    );
    frame.render_widget(
        Gauge::default()
            .ratio(used_pct.clamp(0.0, 1.0))
            .use_unicode(true)
            .gauge_style(Style::new().fg(MEM).bg(Color::Rgb(40, 40, 55)))
            .label(format!("{:.0}%", used_pct * 100.0)),
        ram_g,
    );

    let swap_ratio = if s.swap_total > 0 {
        s.swap_used as f64 / s.swap_total as f64
    } else {
        0.0
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            "Swap ".fg(SWAP).bold(),
            if s.swap_total > 0 {
                format!("{} / {}", fmt_bytes(s.swap_used), fmt_bytes(s.swap_total)).into()
            } else {
                "none".fg(DIM)
            },
        ])),
        swap_l,
    );
    frame.render_widget(
        Gauge::default()
            .ratio(swap_ratio.clamp(0.0, 1.0))
            .use_unicode(true)
            .gauge_style(Style::new().fg(SWAP).bg(Color::Rgb(40, 40, 55)))
            .label(format!("{:.0}%", swap_ratio * 100.0)),
        swap_g,
    );

    if spark.height > 0 {
        let data = tail(&app.mem_hist, spark.width as usize);
        frame.render_widget(
            Sparkline::default()
                .data(data)
                .max(total)
                .absent_value_symbol(" ")
                .style(Style::new().fg(MEM)),
            spark,
        );
    }
}

fn draw_net(frame: &mut Frame, app: &App, area: Rect) {
    let s = &app.snap;
    let b = block(Line::from(vec![
        " Network ".fg(ACCENT).bold(),
        format!("↓ {}", fmt_rate(s.rx_rate)).fg(RX),
        "  ".into(),
        format!("↑ {}", fmt_rate(s.tx_rate)).fg(TX),
        " ".into(),
    ]));
    let inner = b.inner(area);
    frame.render_widget(b, area);
    if inner.height < 2 {
        return;
    }
    let [rx_a, tx_a] =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(inner);

    for (rect, hist, color, label, total) in [
        (rx_a, &app.rx_hist, RX, "rx", s.rx_total),
        (tx_a, &app.tx_hist, TX, "tx", s.tx_total),
    ] {
        if rect.height == 0 {
            continue;
        }
        let [lbl, graph] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(rect);
        let peak = hist.iter().copied().max().unwrap_or(0);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                format!("{label} ").fg(color).bold(),
                format!("peak {}", fmt_rate(peak)).fg(DIM),
                format!("  total {}", fmt_bytes(total)).fg(DIM),
            ])),
            lbl,
        );
        if graph.height > 0 {
            frame.render_widget(
                Sparkline::default()
                    .data(tail(hist, graph.width as usize))
                    .absent_value_symbol(" ")
                    .style(Style::new().fg(color)),
                graph,
            );
        }
    }
}

fn draw_disks(frame: &mut Frame, app: &App, area: Rect) {
    let s = &app.snap;
    let b = block(title("Disks", format!("{}", s.disks.len()).fg(DIM)));
    let inner = b.inner(area);
    frame.render_widget(b, area);
    if inner.height == 0 || s.disks.is_empty() {
        return;
    }
    // Two rows per disk (label + bar) when it fits, otherwise one compact row.
    let two_rows = s.disks.len() * 2 <= inner.height as usize;
    let per: u16 = if two_rows { 2 } else { 1 };
    let rows =
        Layout::vertical(vec![Constraint::Length(per); (inner.height / per) as usize]).split(inner);
    for (d, rect) in s.disks.iter().zip(rows.iter()) {
        let ratio = if d.total > 0 {
            d.used as f64 / d.total as f64
        } else {
            0.0
        };
        let pct = ratio * 100.0;
        let gauge = LineGauge::default()
            .ratio(ratio.clamp(0.0, 1.0))
            .filled_symbol(symbols::line::THICK_HORIZONTAL)
            .unfilled_symbol(symbols::line::THICK_HORIZONTAL)
            .filled_style(Style::new().fg(heat(pct)))
            .unfilled_style(Style::new().fg(Color::Rgb(45, 45, 60)));
        if two_rows {
            let [lbl, bar] =
                Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(*rect);
            let mount_w = rect.width.saturating_sub(24) as usize;
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    ellipsize(&d.mount, mount_w.max(4)).bold(),
                    " ".into(),
                    format!("{} / {}", fmt_bytes(d.used), fmt_bytes(d.total)).fg(DIM),
                    format!("  {pct:.0}%").fg(heat(pct)),
                ])),
                lbl,
            );
            frame.render_widget(gauge.label(""), bar);
        } else {
            let label = Line::from(vec![
                format!("{:<10}", ellipsize(&d.mount, 10)).into(),
                format!("{pct:>3.0}% ").fg(heat(pct)),
            ]);
            frame.render_widget(gauge.label(label), *rect);
        }
    }
}

fn draw_procs(frame: &mut Frame, app: &mut App, area: Rect) {
    let s = &app.snap;
    let arrow = if app.sort_desc { "▼" } else { "▲" };
    let has_threads = s.procs.iter().any(|p| p.threads > 0);
    let mut detail = vec![
        format!("{}", s.procs.len()).bold(),
        " total · ".fg(DIM),
        format!("{}", s.n_running).fg(Color::Green),
        " running · ".fg(DIM),
    ];
    if has_threads {
        detail.push(format!("{}", s.n_threads).into());
        detail.push(" threads · ".fg(DIM));
    }
    detail.push("sort ".fg(DIM));
    detail.push(format!("{}{arrow}", app.sort.label()).fg(ACCENT));
    if !app.filter.is_empty() || app.mode == Mode::Filter {
        detail.push(" · filter ".fg(DIM));
        detail.push(format!("/{}", app.filter).fg(Color::Yellow).bold());
        if app.mode == Mode::Filter {
            detail.push("▏".fg(Color::Yellow));
        }
    }
    let mut t = title("Processes", "");
    t.spans.truncate(3);
    t.spans.extend(detail);
    t.spans.push(" ".into());
    let b = block(t);
    let inner = b.inner(area);
    frame.render_widget(b, area);
    if inner.height < 2 {
        return;
    }

    let fixed: u16 = 7 + 10 + 6 + 6 + 7 + 9 + 1 + 2 + if has_threads { 6 } else { 0 };
    let name_w = inner.width.saturating_sub(fixed + 8) as usize;
    let header_style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
    let col = |name: &'static str, by: SortBy| {
        if app.sort == by {
            Cell::from(format!("{name}{arrow}")).style(header_style.fg(Color::Yellow))
        } else {
            Cell::from(name).style(header_style)
        }
    };
    let mut header_cells = vec![
        col("PID", SortBy::Pid),
        col("USER", SortBy::User),
        col("CPU%", SortBy::Cpu),
        col("MEM%", SortBy::Mem),
        Cell::from("RSS").style(header_style),
    ];
    if has_threads {
        header_cells.push(Cell::from("THR").style(header_style));
    }
    header_cells.extend([
        col("TIME", SortBy::Time),
        Cell::from("S").style(header_style),
        col(if app.show_cmd { "COMMAND" } else { "NAME" }, SortBy::Name),
    ]);
    let header = Row::new(header_cells).style(Style::new().bg(Color::Rgb(30, 32, 44)));

    let rows: Vec<Row> = app
        .visible
        .iter()
        .take(inner.height as usize + app.table.offset())
        .map(|p| {
            let cpu = p.cpu as f64;
            let mem = p.mem_pct as f64;
            let label = if app.show_cmd { &p.cmd } else { &p.name };
            let state = status_code(&p.status);
            let mut cells = vec![
                Cell::from(format!("{:>7}", p.pid)).style(Style::new().fg(DIM)),
                Cell::from(ellipsize(&p.user, 10)),
                Cell::from(format!("{cpu:>6.1}")).style(Style::new().fg(heat(cpu * 1.5))),
                Cell::from(format!("{mem:>6.1}")).style(Style::new().fg(heat(mem * 3.0))),
                Cell::from(format!("{:>7}", fmt_bytes(p.mem))),
            ];
            if has_threads {
                cells.push(Cell::from(format!("{:>5}", p.threads)).style(Style::new().fg(DIM)));
            }
            cells.extend([
                Cell::from(format!("{:>9}", fmt_duration(p.run_time))).style(Style::new().fg(DIM)),
                Cell::from(state.to_string()).style(Style::new().fg(status_color(state))),
                Cell::from(ellipsize(label, name_w.max(4))),
            ]);
            Row::new(cells)
        })
        .collect();

    let mut widths = vec![
        Constraint::Length(7),
        Constraint::Length(10),
        Constraint::Length(6),
        Constraint::Length(6),
        Constraint::Length(7),
    ];
    if has_threads {
        widths.push(Constraint::Length(5));
    }
    widths.extend([
        Constraint::Length(9),
        Constraint::Length(1),
        Constraint::Min(8),
    ]);
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(
            Style::new()
                .bg(Color::Rgb(50, 60, 90))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(table, inner, &mut app.table);
}

fn status_color(state: char) -> Color {
    match state {
        'R' => Color::Green,
        'D' => Color::Yellow,
        'T' | 't' => Color::Magenta,
        'Z' | 'X' => Color::Red,
        _ => DIM,
    }
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let key = |k: &'static str, what: &'static str| -> Vec<Span<'static>> {
        vec![
            format!(" {k} ").fg(Color::Black).bg(ACCENT).bold(),
            format!(" {what} ").fg(DIM),
        ]
    };
    let mut spans: Vec<Span> = Vec::new();
    match app.mode {
        Mode::Filter => {
            spans.extend(key("Enter", "apply"));
            spans.extend(key("Esc", "clear"));
            spans.push(format!("  filter: /{}▏", app.filter).fg(Color::Yellow));
        }
        _ => {
            for (k, w) in [
                ("q", "quit"),
                ("↑↓", "move"),
                ("c m p n u t", "sort"),
                ("r", "reverse"),
                ("/", "filter"),
                ("e", "cmd"),
                ("␣", "pause"),
                ("+ -", "rate"),
                ("x", "term"),
                ("X", "kill"),
                ("?", "help"),
            ] {
                spans.extend(key(k, w));
            }
        }
    }
    let left = Line::from(spans);
    let left_w = left.width() as u16;
    frame.render_widget(Paragraph::new(left), area);

    let right = if let Some(msg) = app.status_text() {
        Line::from(format!(" {msg} ").fg(Color::Black).bg(Color::Yellow))
    } else {
        Line::from(
            format!(
                "sample {}ms · every {}ms ",
                app.snap.sample_ms,
                app.interval.as_millis()
            )
            .fg(DIM),
        )
    };
    let rw = right.width() as u16;
    if left_w + rw < area.width {
        let r = Rect {
            x: area.x + area.width - rw,
            width: rw,
            ..area
        };
        frame.render_widget(Paragraph::new(right).alignment(Alignment::Right), r);
    }
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let lines: Vec<Line> = [
        ("q / Esc", "quit"),
        ("↑ ↓ / j k", "move selection"),
        ("PgUp PgDn / Ctrl-u Ctrl-d", "move 20 rows"),
        ("g / G", "first / last"),
        ("c m p n u t", "sort by CPU, memory, PID, name, user, time"),
        ("r", "reverse sort order"),
        ("/", "filter by name, command, user or PID"),
        ("e", "toggle name / full command line"),
        ("space", "pause / resume sampling"),
        ("+ / -", "faster / slower refresh (250 ms steps)"),
        ("x / Del", "send SIGTERM to selected process"),
        ("X / F9", "send SIGKILL to selected process"),
        ("? / F1", "this help"),
    ]
    .into_iter()
    .map(|(k, v)| {
        Line::from(vec![
            format!("{k:>26}").fg(ACCENT).bold(),
            "  ".into(),
            v.into(),
        ])
    })
    .collect();
    let h = lines.len() as u16 + 2;
    let w = 70;
    let popup = centered(area, w, h);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(block(title("Help", "press any key".fg(DIM)))),
        popup,
    );
}

fn draw_confirm(frame: &mut Frame, area: Rect, pid: u32, name: &str, force: bool) {
    let sig = if force { "SIGKILL" } else { "SIGTERM" };
    let text = Line::from(vec![
        "Send ".into(),
        sig.fg(Color::Red).bold(),
        " to ".into(),
        name.bold(),
        format!(" ({pid})? ").into(),
        "y".fg(Color::Green).bold(),
        "/".fg(DIM),
        "n".fg(Color::Red).bold(),
    ]);
    let w = (text.width() as u16 + 6).min(area.width);
    let popup = centered(area, w, 3);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(block(title("Confirm", "")).border_style(Style::new().fg(Color::Red))),
        popup,
    );
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

/// Last `width` samples, left-padded with `None` so the series is right-aligned.
fn tail(hist: &std::collections::VecDeque<u64>, width: usize) -> Vec<Option<u64>> {
    let start = hist.len().saturating_sub(width);
    let pad = width.saturating_sub(hist.len());
    std::iter::repeat_n(None, pad)
        .chain(hist.iter().skip(start).map(|v| Some(*v)))
        .collect()
}
