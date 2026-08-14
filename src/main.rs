use clap::Parser;
use color_eyre::{config::HookBuilder, eyre, Result};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use env_logger;
use recon::{App, Config};
use log::debug;
use ratatui::{prelude::CrosstermBackend, Terminal};
use std::io::{self, Stdout};
use std::panic;

fn main() -> Result<()> {
    install_error_hooks()?;

    setup_logging();
    let config = Config::parse();
    debug!("{:?}", config);

    let terminal = init_terminal()?;
    App::new(&config).run(terminal)?;
    restore_terminal()?;

    Ok(())
}

//===================================================================================

/// Install `color_eyre` panic and error hooks
///
/// The hooks restore the terminal to a usable state before printing the error message.
fn install_error_hooks() -> Result<()> {
    let (panic, error) = HookBuilder::default().into_hooks();
    let panic = panic.into_panic_hook();
    let error = error.into_eyre_hook();
    eyre::set_hook(Box::new(move |e| {
        let _ = restore_terminal();
        error(e)
    }))?;
    panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        panic(info);
    }));
    Ok(())
}

fn setup_logging() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .format_target(false)
        .format_timestamp(None)
        .init();
}

fn init_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    Ok(terminal)
}

fn restore_terminal() -> Result<()> {
    disable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, LeaveAlternateScreen, DisableMouseCapture)?;
    // terminal.show_cursor()?;
    Ok(())
}
