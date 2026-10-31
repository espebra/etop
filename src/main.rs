mod app;
mod sys;
mod ui;
mod util;

use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyEventKind};

use app::App;
use sys::Sampler;

const USAGE: &str = "etop - a visual system monitor for the terminal

USAGE:
    etop [OPTIONS]

OPTIONS:
    -r, --rate <MS>    Refresh interval in milliseconds (default 1000, min 250)
    -h, --help         Print this help
    -V, --version      Print version
";

fn main() {
    let mut interval = Duration::from_millis(1000);
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
    loop {
        app.poll();
        terminal.draw(|frame| ui::draw(frame, app))?;

        // Wake up at least ten times a second so new samples and status
        // timeouts are reflected promptly, without burning CPU.
        if event::poll(Duration::from_millis(100))? {
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
