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
    pub show_ifaces: bool,
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
            show_ifaces: false,
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
            KeyCode::Char('i') => self.show_ifaces = !self.show_ifaces,
            KeyCode::Char(' ') => {
                self.paused = !self.paused;
                let msg = if self.paused { "Paused" } else { "Resumed" };
                self.set_status(msg.into());
            }
            KeyCode::Char('+') | KeyCode::Char('=') => self.change_interval(-250),
            KeyCode::Char('-') | KeyCode::Char('_') => self.change_interval(250),
            _ => {}
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{Receiver, Sender};

    fn proc(pid: u32, name: &str, user: &str, cpu: f32, mem: u64, run_time: u64) -> ProcInfo {
        ProcInfo {
            pid,
            name: name.into(),
            cmd: format!("/usr/bin/{name} --flag"),
            user: user.into(),
            cpu,
            mem,
            mem_pct: 0.0,
            run_time,
            status: "Sleeping".into(),
            threads: 1,
        }
    }

    fn procs() -> Vec<ProcInfo> {
        vec![
            proc(1, "init", "root", 0.5, 100, 9000),
            proc(42, "nginx", "www", 12.0, 500, 300),
            proc(7, "bash", "espen", 3.0, 200, 60),
            proc(300, "Zed", "espen", 3.0, 900, 10),
        ]
    }

    struct Harness {
        app: App,
        snaps: Sender<Snapshot>,
        cmds: Receiver<Command>,
    }

    fn harness() -> Harness {
        let (sampler, snaps, cmds) = Sampler::fake(StaticInfo::default());
        let app = App::new(sampler, Duration::from_millis(2000));
        Harness { app, snaps, cmds }
    }

    fn feed(h: &mut Harness, snap: Snapshot) {
        h.snaps.send(snap).unwrap();
        h.app.poll();
    }

    fn snap_with_procs() -> Snapshot {
        Snapshot {
            procs: procs(),
            ..Default::default()
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn visible_pids(app: &App) -> Vec<u32> {
        app.visible.iter().map(|p| p.pid).collect()
    }

    #[test]
    fn push_caps_history_length() {
        let mut hist = VecDeque::new();
        for i in 0..(HISTORY as u64 + 5) {
            push(&mut hist, i);
        }
        assert_eq!(hist.len(), HISTORY);
        assert_eq!(hist.front(), Some(&5));
        assert_eq!(hist.back(), Some(&(HISTORY as u64 + 4)));
    }

    #[test]
    fn poll_without_snapshot_is_noop() {
        let mut h = harness();
        h.app.poll();
        assert!(h.app.cpu_hist.is_empty());
        assert!(h.app.visible.is_empty());
    }

    #[test]
    fn poll_folds_snapshot_into_history() {
        let mut h = harness();
        feed(
            &mut h,
            Snapshot {
                cpu_total: 42.5,
                cpu_cores: vec![10.0, 75.0],
                mem_used: 1234,
                rx_rate: 10,
                tx_rate: 20,
                ..Default::default()
            },
        );
        assert_eq!(h.app.cpu_hist.back(), Some(&42.5));
        assert_eq!(h.app.mem_hist.back(), Some(&1234));
        assert_eq!(h.app.rx_hist.back(), Some(&10));
        assert_eq!(h.app.tx_hist.back(), Some(&20));
        assert_eq!(h.app.core_hist.len(), 2);
        assert_eq!(h.app.core_hist[0].back(), Some(&10));
        assert_eq!(h.app.core_hist[1].back(), Some(&75));
        assert_eq!(h.app.snap.cpu_total, 42.5);
    }

    #[test]
    fn poll_resets_core_history_when_core_count_changes() {
        let mut h = harness();
        feed(
            &mut h,
            Snapshot {
                cpu_cores: vec![1.0, 2.0],
                ..Default::default()
            },
        );
        feed(
            &mut h,
            Snapshot {
                cpu_cores: vec![1.0, 2.0, 3.0],
                ..Default::default()
            },
        );
        assert_eq!(h.app.core_hist.len(), 3);
        assert!(h.app.core_hist.iter().all(|c| c.len() == 1));
    }

    #[test]
    fn paused_app_drops_snapshots() {
        let mut h = harness();
        h.app.handle_key(key(KeyCode::Char(' ')));
        assert!(h.app.paused);
        assert_eq!(h.app.status_text(), Some("Paused"));

        feed(&mut h, snap_with_procs());
        assert!(h.app.cpu_hist.is_empty());
        assert!(h.app.visible.is_empty());

        h.app.handle_key(key(KeyCode::Char(' ')));
        assert!(!h.app.paused);
        assert_eq!(h.app.status_text(), Some("Resumed"));
        feed(&mut h, snap_with_procs());
        assert_eq!(h.app.visible.len(), 4);
    }

    #[test]
    fn default_sort_is_cpu_descending_with_pid_tiebreak() {
        let mut h = harness();
        feed(&mut h, snap_with_procs());
        // bash(7) and Zed(300) tie on CPU; descending order reverses the pid
        // tiebreak too, so the higher pid comes first.
        assert_eq!(visible_pids(&h.app), vec![42, 300, 7, 1]);
        assert_eq!(h.app.table.selected(), Some(0));
    }

    #[test]
    fn sort_keys_switch_column_and_toggle_direction() {
        let mut h = harness();
        feed(&mut h, snap_with_procs());

        h.app.handle_key(key(KeyCode::Char('p')));
        assert_eq!(h.app.sort, SortBy::Pid);
        assert!(!h.app.sort_desc);
        assert_eq!(visible_pids(&h.app), vec![1, 7, 42, 300]);

        // Same key again flips the direction.
        h.app.handle_key(key(KeyCode::Char('p')));
        assert!(h.app.sort_desc);
        assert_eq!(visible_pids(&h.app), vec![300, 42, 7, 1]);

        h.app.handle_key(key(KeyCode::Char('m')));
        assert_eq!(h.app.sort, SortBy::Mem);
        assert!(h.app.sort_desc);
        assert_eq!(visible_pids(&h.app), vec![300, 42, 7, 1]);

        h.app.handle_key(key(KeyCode::Char('t')));
        assert_eq!(visible_pids(&h.app), vec![1, 42, 7, 300]);

        h.app.handle_key(key(KeyCode::Char('u')));
        assert!(!h.app.sort_desc);
        assert_eq!(visible_pids(&h.app), vec![7, 300, 1, 42]);

        h.app.handle_key(key(KeyCode::Char('r')));
        assert!(h.app.sort_desc);
        assert_eq!(visible_pids(&h.app), vec![42, 1, 300, 7]);
    }

    #[test]
    fn name_sort_is_case_insensitive() {
        let mut h = harness();
        feed(&mut h, snap_with_procs());
        h.app.handle_key(key(KeyCode::Char('n')));
        assert_eq!(visible_pids(&h.app), vec![7, 1, 42, 300]);
    }

    #[test]
    fn filter_matches_name_cmd_user_and_pid() {
        let mut h = harness();
        feed(&mut h, snap_with_procs());

        h.app.handle_key(key(KeyCode::Char('/')));
        assert_eq!(h.app.mode, Mode::Filter);

        for c in "NGI".chars() {
            h.app.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(h.app.filter, "NGI");
        assert_eq!(visible_pids(&h.app), vec![42]);

        h.app.handle_key(ctrl('u'));
        assert!(h.app.filter.is_empty());
        assert_eq!(h.app.visible.len(), 4);

        for c in "espen".chars() {
            h.app.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(visible_pids(&h.app), vec![300, 7]);

        h.app.handle_key(ctrl('u'));
        for c in "300".chars() {
            h.app.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(visible_pids(&h.app), vec![300]);

        h.app.handle_key(ctrl('u'));
        for c in "--flag".chars() {
            h.app.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(h.app.visible.len(), 4);

        h.app
            .handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert!(h.app.visible.is_empty());
        assert_eq!(h.app.table.selected(), None);

        h.app.handle_key(key(KeyCode::Backspace));
        assert_eq!(h.app.filter, "--flag");
        assert_eq!(h.app.table.selected(), Some(0));
    }

    #[test]
    fn filter_enter_keeps_filter_and_esc_clears_it() {
        let mut h = harness();
        feed(&mut h, snap_with_procs());

        h.app.handle_key(key(KeyCode::Char('/')));
        for c in "bash".chars() {
            h.app.handle_key(key(KeyCode::Char(c)));
        }
        h.app.handle_key(key(KeyCode::Enter));
        assert_eq!(h.app.mode, Mode::Normal);
        assert_eq!(h.app.filter, "bash");
        assert_eq!(visible_pids(&h.app), vec![7]);

        h.app.handle_key(key(KeyCode::Char('/')));
        h.app.handle_key(key(KeyCode::Esc));
        assert_eq!(h.app.mode, Mode::Normal);
        assert!(h.app.filter.is_empty());
        assert_eq!(h.app.visible.len(), 4);
    }

    #[test]
    fn filter_persists_across_snapshots() {
        let mut h = harness();
        feed(&mut h, snap_with_procs());
        h.app.handle_key(key(KeyCode::Char('/')));
        h.app.handle_key(key(KeyCode::Char('w')));
        h.app.handle_key(key(KeyCode::Enter));
        feed(&mut h, snap_with_procs());
        assert_eq!(visible_pids(&h.app), vec![42]);
    }

    #[test]
    fn cursor_moves_and_clamps() {
        let mut h = harness();
        feed(&mut h, snap_with_procs());

        h.app.handle_key(key(KeyCode::Down));
        assert_eq!(h.app.table.selected(), Some(1));
        h.app.handle_key(key(KeyCode::Char('j')));
        assert_eq!(h.app.table.selected(), Some(2));
        h.app.handle_key(key(KeyCode::PageDown));
        assert_eq!(h.app.table.selected(), Some(3));
        h.app.handle_key(key(KeyCode::Char('k')));
        assert_eq!(h.app.table.selected(), Some(2));
        h.app.handle_key(key(KeyCode::PageUp));
        assert_eq!(h.app.table.selected(), Some(0));
        h.app.handle_key(key(KeyCode::Up));
        assert_eq!(h.app.table.selected(), Some(0));
        h.app.handle_key(key(KeyCode::Char('G')));
        assert_eq!(h.app.table.selected(), Some(3));
        h.app.handle_key(key(KeyCode::Char('g')));
        assert_eq!(h.app.table.selected(), Some(0));
        h.app.handle_key(ctrl('d'));
        assert_eq!(h.app.table.selected(), Some(3));
        h.app.handle_key(ctrl('u'));
        assert_eq!(h.app.table.selected(), Some(0));
    }

    #[test]
    fn cursor_with_no_rows_is_none() {
        let mut h = harness();
        h.app.handle_key(key(KeyCode::Down));
        assert_eq!(h.app.table.selected(), None);
        assert!(h.app.selected().is_none());
    }

    #[test]
    fn selection_follows_process_when_rows_reorder() {
        let mut h = harness();
        feed(&mut h, snap_with_procs());
        h.app.handle_key(key(KeyCode::Down));
        assert_eq!(h.app.selected().map(|p| p.pid), Some(300));

        // nginx drops, Zed rises: cursor should stay on Zed (pid 300).
        let mut procs = procs();
        procs[1].cpu = 0.1;
        procs[3].cpu = 50.0;
        feed(
            &mut h,
            Snapshot {
                procs,
                ..Default::default()
            },
        );
        assert_eq!(visible_pids(&h.app)[0], 300);
        assert_eq!(h.app.selected().map(|p| p.pid), Some(300));
        assert_eq!(h.app.table.selected(), Some(0));
    }

    #[test]
    fn selection_at_top_stays_at_top() {
        let mut h = harness();
        feed(&mut h, snap_with_procs());
        assert_eq!(h.app.selected().map(|p| p.pid), Some(42));

        let mut procs = procs();
        procs[3].cpu = 50.0;
        feed(
            &mut h,
            Snapshot {
                procs,
                ..Default::default()
            },
        );
        assert_eq!(h.app.table.selected(), Some(0));
        assert_eq!(h.app.selected().map(|p| p.pid), Some(300));
    }

    #[test]
    fn selection_clamps_when_selected_process_disappears() {
        let mut h = harness();
        feed(&mut h, snap_with_procs());
        h.app.handle_key(key(KeyCode::Char('G')));
        assert_eq!(h.app.table.selected(), Some(3));

        feed(
            &mut h,
            Snapshot {
                procs: procs()[..2].to_vec(),
                ..Default::default()
            },
        );
        assert_eq!(h.app.table.selected(), Some(1));
        assert!(h.app.selected().is_some());
    }

    #[test]
    fn change_interval_clamps_and_notifies_sampler() {
        let mut h = harness();
        h.app.handle_key(key(KeyCode::Char('+')));
        assert_eq!(h.app.interval, Duration::from_millis(1750));
        assert_eq!(h.app.status_text(), Some("Refresh interval: 1750 ms"));
        assert!(matches!(
            h.cmds.try_recv(),
            Ok(Command::SetInterval(d)) if d == Duration::from_millis(1750)
        ));

        for _ in 0..20 {
            h.app.handle_key(key(KeyCode::Char('=')));
        }
        assert_eq!(h.app.interval, Duration::from_millis(250));

        for _ in 0..100 {
            h.app.handle_key(key(KeyCode::Char('-')));
        }
        assert_eq!(h.app.interval, Duration::from_millis(10_000));
    }

    #[test]
    fn quit_keys_set_should_quit() {
        for k in [key(KeyCode::Char('q')), key(KeyCode::Esc), ctrl('c')] {
            let mut h = harness();
            h.app.handle_key(k);
            assert!(h.app.should_quit, "{k:?}");
        }
    }

    #[test]
    fn help_mode_opens_and_any_key_closes() {
        let mut h = harness();
        h.app.handle_key(key(KeyCode::Char('?')));
        assert_eq!(h.app.mode, Mode::Help);
        h.app.handle_key(key(KeyCode::Char('q')));
        assert_eq!(h.app.mode, Mode::Normal);
        assert!(!h.app.should_quit);

        h.app.handle_key(key(KeyCode::F(1)));
        assert_eq!(h.app.mode, Mode::Help);
    }

    #[test]
    fn toggles_flip_display_flags() {
        let mut h = harness();
        assert!(!h.app.show_cmd);
        assert!(!h.app.show_ifaces);
        h.app.handle_key(key(KeyCode::Char('e')));
        h.app.handle_key(key(KeyCode::Char('i')));
        assert!(h.app.show_cmd);
        assert!(h.app.show_ifaces);
        h.app.handle_key(key(KeyCode::Char('e')));
        assert!(!h.app.show_cmd);
    }

    #[test]
    fn shutdown_sends_stop() {
        let h = harness();
        h.app.shutdown();
        assert!(matches!(h.cmds.try_recv(), Ok(Command::Stop)));
    }

    #[test]
    fn status_text_expires() {
        let mut h = harness();
        assert_eq!(h.app.status_text(), None);
        h.app.set_status("hi".into());
        assert_eq!(h.app.status_text(), Some("hi"));
        h.app.status = Some((
            "old".into(),
            std::time::Instant::now() - Duration::from_secs(5),
        ));
        assert_eq!(h.app.status_text(), None);
    }

    #[test]
    fn sort_labels() {
        assert_eq!(SortBy::Cpu.label(), "CPU");
        assert_eq!(SortBy::Mem.label(), "MEM");
        assert_eq!(SortBy::Pid.label(), "PID");
        assert_eq!(SortBy::Name.label(), "NAME");
        assert_eq!(SortBy::User.label(), "USER");
        assert_eq!(SortBy::Time.label(), "TIME");
    }
}
