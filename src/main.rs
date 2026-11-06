#[cfg(target_os = "macos")]
mod macos;
mod ui;

use gitk_rs::cli;

fn main() {
    let launched = std::time::Instant::now();
    match cli::parse(std::env::args_os().skip(1)) {
        Ok(config) => ui::run(config, launched),
        Err(error) => error.exit(),
    }
}
