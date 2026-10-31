use std::collections::VecDeque;
use std::time::Duration;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::TableState;

use crate::sys::{Command, ProcInfo, Sampler, Snapshot, StaticInfo};

/// Number of samples kept for the history graphs.
pub const HISTORY: usize = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Cpu,
    Mem,
    Pid,
    Name,
    User,
    Time,
}

impl SortBy {
    pub fn label(self) -> &'static str {
        match self {
            SortBy::Cpu => "CPU",
            SortBy::Mem => "MEM",
            SortBy::Pid => "PID",
            SortBy::Name => "NAME",
            SortBy::User => "USER",
            SortBy::Time => "TIME",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Filter,
    ConfirmKill { pid: u32, name: String, force: bool },
    Help,
}

pub struct App {
    pub info: StaticInfo,
    pub snap: Snapshot,
    pub cpu_hist: VecDeque<f64>,
    pub mem_hist: VecDeque<u64>,
    pub rx_hist: VecDeque<u64>,
    pub tx_hist: VecDeque<u64>,
    pub core_hist: Vec<VecDeque<u64>>,
    pub sort: SortBy,
    pub sort_desc: bool,
    pub filter: String,
    pub mode: Mode,
    pub table: TableState,
    pub visible: Vec<ProcInfo>,
    pub show_cmd: bool,
    pub paused: bool,
    pub interval: Duration,
    pub status: Option<(String, std::time::Instant)>,
    pub should_quit: bool,
    sampler: Sampler,
}

impl App {
    pub fn new(sampler: Sampler, interval: Duration) -> Self {
        let info = sampler.info.clone();
        let mut table = TableState::default();
        table.select(Some(0));
        Self {
            info,
            snap: Snapshot::default(),
            cpu_hist: VecDeque::with_capacity(HISTORY),
            mem_hist: VecDeque::with_capacity(HISTORY),
            rx_hist: VecDeque::with_capacity(HISTORY),
            tx_hist: VecDeque::with_capacity(HISTORY),
            core_hist: Vec::new(),
            sort: SortBy::Cpu,
            sort_desc: true,
            filter: String::new(),
            mode: Mode::Normal,
            table,
            visible: Vec::new(),
            show_cmd: false,
            paused: false,
            interval,
            status: None,
            should_quit: false,
            sampler,
        }
    }

    /// Pull any new snapshot from the sampler and fold it into history.
    pub fn poll(&mut self) {
        let Some(snap) = self.sampler.latest() else {
            return;
        };
        if let Some(msg) = &snap.message {
            self.set_status(msg.clone());
        }
        if self.paused {
            return;
        }
        push(&mut self.cpu_hist, snap.cpu_total as f64);
        push(&mut self.mem_hist, snap.mem_used);
        push(&mut self.rx_hist, snap.rx_rate);
        push(&mut self.tx_hist, snap.tx_rate);
        if self.core_hist.len() != snap.cpu_cores.len() {
            self.core_hist = snap
                .cpu_cores
                .iter()
                .map(|_| VecDeque::with_capacity(HISTORY))
                .collect();
        }
        for (h, v) in self.core_hist.iter_mut().zip(&snap.cpu_cores) {
            push(h, *v as u64);
        }
        self.snap = snap;
        self.rebuild_visible();
    }

    pub fn set_status(&mut self, msg: String) {
        self.status = Some((msg, std::time::Instant::now()));
    }

    pub fn status_text(&self) -> Option<&str> {
        match &self.status {
            Some((m, at)) if at.elapsed() < Duration::from_secs(4) => Some(m),
            _ => None,
        }
    }

    pub fn selected(&self) -> Option<&ProcInfo> {
        self.table.selected().and_then(|i| self.visible.get(i))
    }

    fn rebuild_visible(&mut self) {
        let selected_pid = self.selected().map(|p| p.pid);
        let needle = self.filter.to_lowercase();
        let mut v: Vec<ProcInfo> = self
            .snap
            .procs
            .iter()
            .filter(|p| {
                needle.is_empty()
                    || p.name.to_lowercase().contains(&needle)
                    || p.cmd.to_lowercase().contains(&needle)
                    || p.user.to_lowercase().contains(&needle)
                    || p.pid.to_string() == needle
            })
            .cloned()
            .collect();

        let desc = self.sort_desc;
        v.sort_by(|a, b| {
            let ord = match self.sort {
                SortBy::Cpu => a.cpu.total_cmp(&b.cpu),
                SortBy::Mem => a.mem.cmp(&b.mem),
                SortBy::Pid => a.pid.cmp(&b.pid),
                SortBy::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortBy::User => a.user.cmp(&b.user),
                SortBy::Time => a.run_time.cmp(&b.run_time),
            };
            let ord = ord.then_with(|| a.pid.cmp(&b.pid));
            if desc { ord.reverse() } else { ord }
        });
        self.visible = v;

        // Follow the selected process as rows reorder, unless the cursor is
        // parked at the top, where users expect to always see the top entry.
        let idx = self
            .table
            .selected()
            .filter(|i| *i > 0)
            .and(selected_pid)
            .and_then(|pid| self.visible.iter().position(|p| p.pid == pid))
            .or_else(|| self.table.selected())
            .map(|i| i.min(self.visible.len().saturating_sub(1)));
        self.table.select(if self.visible.is_empty() {
            None
        } else {
            idx.or(Some(0))
        });
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match self.mode.clone() {
            Mode::Filter => self.key_filter(key),
            Mode::ConfirmKill { pid, force, .. } => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                        let cmd = if force {
                            Command::Kill(pid)
                        } else {
                            Command::Terminate(pid)
                        };
                        self.sampler.send(cmd);
                    }
                    _ => {}
                }
                self.mode = Mode::Normal;
            }
            Mode::Help => {
                self.mode = Mode::Normal;
            }
            Mode::Normal => self.key_normal(key),
        }
    }

    fn key_filter(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.filter.clear();
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => self.mode = Mode::Normal,
            KeyCode::Backspace => {
                self.filter.pop();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.filter.clear();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.filter.push(c);
            }
            _ => {}
        }
        self.rebuild_visible();
    }

    fn key_normal(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('c') if ctrl => self.should_quit = true,
            KeyCode::Char('?') | KeyCode::F(1) => self.mode = Mode::Help,

            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::PageDown | KeyCode::Char('d') if !ctrl || key.code == KeyCode::PageDown => {
                self.move_cursor(20)
            }
            KeyCode::Char('d') if ctrl => self.move_cursor(20),
            KeyCode::PageUp => self.move_cursor(-20),
            KeyCode::Char('u') if ctrl => self.move_cursor(-20),
            KeyCode::Home | KeyCode::Char('g') => self.table.select(Some(0)),
            KeyCode::End | KeyCode::Char('G') => self
                .table
                .select(Some(self.visible.len().saturating_sub(1))),

            KeyCode::Char('c') => self.set_sort(SortBy::Cpu),
            KeyCode::Char('m') => self.set_sort(SortBy::Mem),
            KeyCode::Char('p') => self.set_sort(SortBy::Pid),
            KeyCode::Char('n') => self.set_sort(SortBy::Name),
            KeyCode::Char('u') => self.set_sort(SortBy::User),
            KeyCode::Char('t') => self.set_sort(SortBy::Time),
            KeyCode::Char('r') => {
                self.sort_desc = !self.sort_desc;
                self.rebuild_visible();
            }

            KeyCode::Char('/') => self.mode = Mode::Filter,
            KeyCode::Char('e') => self.show_cmd = !self.show_cmd,
            KeyCode::Char(' ') => {
                self.paused = !self.paused;
                let msg = if self.paused { "Paused" } else { "Resumed" };
                self.set_status(msg.into());
            }
            KeyCode::Char('+') | KeyCode::Char('=') => self.change_interval(-250),
            KeyCode::Char('-') | KeyCode::Char('_') => self.change_interval(250),

            KeyCode::Char('x') | KeyCode::Delete => self.ask_kill(false),
            KeyCode::Char('X') | KeyCode::F(9) => self.ask_kill(true),
            _ => {}
        }
    }

    fn ask_kill(&mut self, force: bool) {
        if let Some(p) = self.selected() {
            self.mode = Mode::ConfirmKill {
                pid: p.pid,
                name: p.name.clone(),
                force,
            };
        }
    }

    fn set_sort(&mut self, by: SortBy) {
        if self.sort == by {
            self.sort_desc = !self.sort_desc;
        } else {
            self.sort = by;
            self.sort_desc = !matches!(by, SortBy::Pid | SortBy::Name | SortBy::User);
        }
        self.rebuild_visible();
    }

    fn move_cursor(&mut self, delta: i64) {
        if self.visible.is_empty() {
            self.table.select(None);
            return;
        }
        let cur = self.table.selected().unwrap_or(0) as i64;
        let max = self.visible.len() as i64 - 1;
        let next = (cur + delta).clamp(0, max);
        self.table.select(Some(next as usize));
    }

    fn change_interval(&mut self, delta_ms: i64) {
        let cur = self.interval.as_millis() as i64;
        let next = (cur + delta_ms).clamp(250, 10_000);
        self.interval = Duration::from_millis(next as u64);
        self.sampler.send(Command::SetInterval(self.interval));
        self.set_status(format!("Refresh interval: {} ms", next));
    }

    pub fn shutdown(&self) {
        self.sampler.send(Command::Stop);
    }
}

fn push<T>(hist: &mut VecDeque<T>, v: T) {
    if hist.len() >= HISTORY {
        hist.pop_front();
    }
    hist.push_back(v);
}
