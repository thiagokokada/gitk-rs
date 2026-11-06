#[cfg(target_os = "macos")]
mod macos;
mod ui;

use gitk_rs::cli;

fn main() {
    let launched = std::time::Instant::now();
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match cli::parse(args.iter().cloned()) {
        Ok(_) => {
            // Handle help/version and invalid arguments before touching configuration files.
            let preferences = gitk_rs::preferences::Preferences::load().unwrap_or_else(|error| {
                eprintln!("gitk-rs: preferences: {error:#}");
                gitk_rs::preferences::Preferences::default()
            });
            match cli::parse_with_preferences(args, &preferences) {
                Ok(config) => ui::run(config, preferences, launched),
                Err(error) => error.exit(),
            }
        }
        Err(error) => error.exit(),
    }
}
