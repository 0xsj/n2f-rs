use n2f_rs::{root, shared::env};
use std::io::{self, IsTerminal};
fn main() {
    let lookup = match env::os() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("configuration source unavailable");
            std::process::exit(2)
        }
    };
    let terminal = io::stdout().is_terminal();
    let code = root::run(lookup, Box::new(io::stdout()), &mut io::stderr(), terminal);
    std::process::exit(code);
}
