mod ui;

use gitk_rs::cli::{self, Command};

fn main() {
    let launched = std::time::Instant::now();
    match cli::parse(std::env::args_os().skip(1)) {
        Ok(Command::Languages) => print!("{}", gitk_rs::syntax::inventory()),
        Ok(Command::Open(config)) => ui::run(config, launched),
        Err(error) => error.exit(),
    }
}
