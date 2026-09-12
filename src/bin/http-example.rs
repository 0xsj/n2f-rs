use n2f_rs::{root::http::run_http, shared::env};
fn main() {
    let lookup = match env::os() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("configuration source unavailable");
            std::process::exit(2)
        }
    };
    std::process::exit(run_http(lookup));
}
