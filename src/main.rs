#![forbid(unsafe_code)]

mod app;
mod sys;
mod ui;
mod util;

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyEventKind};

use app::App;
use sys::Sampler;

const USAGE: &str = "etop - a visual system monitor for the terminal

USAGE:
    etop [OPTIONS]

OPTIONS:
    -r, --rate <MS>    Refresh interval in milliseconds (default 2000, min 250),
                       aligned to the wall clock
    -h, --help         Print this help
    -V, --version      Print version
";

fn main() {
    let mut interval = Duration::from_millis(2000);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return;
            }
            "-V" | "--version" => {
                println!("etop {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "-r" | "--rate" => {
                let ms = args
                    .next()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or_else(|| {
                        eprintln!("error: --rate needs a number of milliseconds");
                        std::process::exit(2);
                    });
                interval = Duration::from_millis(ms.max(250));
            }
            other => {
                eprintln!("error: unknown argument '{other}'\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    let sampler = match Sampler::spawn(interval) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: failed to initialise system sampling: {e}");
            std::process::exit(1);
        }
    };
    let mut app = App::new(sampler, interval);

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    app.shutdown();
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> std::io::Result<()> {
    // When the sampler is next expected to produce a snapshot. It wakes on
    // wall-clock multiples of the interval, so the screen is redrawn at the
    // same moment in every etop window.
    let mut next_tick = Instant::now() + sys::until_next_tick(app.interval);
    loop {
        let now = Instant::now();
        if app.poll() || now > next_tick + app.interval {
            next_tick = now + sys::until_next_tick(app.interval);
        }
        terminal.draw(|frame| ui::draw(frame, app))?;

        // Wake up at least ten times a second so status timeouts are
        // reflected promptly, without burning CPU, and on each wall-clock
        // second so the clock ticks on time. Once a tick is due, poll
        // briefly so the new snapshot is shown as soon as it is ready.
        let timeout = next_tick
            .saturating_duration_since(Instant::now())
            .min(sys::until_next_tick(Duration::from_secs(1)))
            .clamp(Duration::from_millis(5), Duration::from_millis(100));
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.handle_key(key),
                _ => {}
            }
        }
        if app.should_quit {
            return Ok(());
        }
    }
}
