mod ui;

use gitk_rs::cli::{self, Command};

fn main() {
    let launched = std::time::Instant::now();
    match cli::parse(std::env::args_os().skip(1)) {
        Ok(Command::Help) => println!("{}", cli::HELP),
        Ok(Command::Version) => println!("gitk-rs {}", env!("CARGO_PKG_VERSION")),
        Ok(Command::Open(config)) => ui::run(config, launched),
        Err(error) => {
            eprintln!("gitk-rs: {error}\nUse --help for usage.");
            std::process::exit(2);
        }
    }
}
