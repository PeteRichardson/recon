use color_eyre::{Result, config::HookBuilder, eyre};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, prelude::CrosstermBackend};
use recon::{App, Config};
use std::io::{self, Stdout};
use std::panic;

fn main() -> Result<()> {
    install_error_hooks()?;

    setup_logging();
    // Before `init_terminal`, and that ordering is load-bearing rather than
    // incidental. A config error printed after raw mode and the alternate
    // screen are in place is wiped off the screen a frame later, so the only
    // place a bad `config.toml` can be reported legibly is here — on stderr,
    // on the normal screen, where the existing `[DEBUG] Config { .. }` line
    // already goes. Hence a hard failure rather than a warning: "warn and
    // carry on" would be "carry on silently".
    let config = Config::load()?;

    // Before the terminal, for the same reason the config error is: this prints
    // a snippet to be copied out of the scrollback, and the alternate screen
    // would take it away the moment it was drawn. It also *only* prints —
    // recon never writes `config.toml` (see `Cargo.toml`) and will not write a
    // shell rc either, so there is nothing to undo afterwards.
    if let Some(flavour) = &config.print_editor_config {
        print!(
            "{}",
            recon::editor::print_editor_config(
                flavour,
                std::env::var("TERM_PROGRAM").ok().as_deref(),
            )?
        );
        return Ok(());
    }

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
