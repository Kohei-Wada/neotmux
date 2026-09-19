mod app;
mod input;
mod render;

fn main() -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let result = app::run(&mut terminal);
    ratatui::restore();
    result
}
