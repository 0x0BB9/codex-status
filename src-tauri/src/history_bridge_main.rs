mod history_bridge;

fn main() {
    match history_bridge::run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("History compatibility bridge failed: {error}");
            std::process::exit(1);
        }
    }
}
