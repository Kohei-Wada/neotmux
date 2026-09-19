mod app;
mod input;
mod render;
mod signals;

fn main() -> anyhow::Result<()> {
    let shutdown = signals::ShutdownSignal::install()?;
    let mut terminal = ratatui::init();
    let result = app::run(&mut terminal, &shutdown);
    ratatui::restore();
    match result? {
        Some(signal) => signals::die_of(signal),
        None => Ok(()),
    }
}
