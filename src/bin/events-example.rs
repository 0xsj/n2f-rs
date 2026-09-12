fn main() {
    let lookup = match n2f_rs::shared::env::os() {
        Ok(v) => v,
        Err(_) => std::process::exit(2),
    };
    std::process::exit(n2f_rs::root::events::run(lookup));
}
