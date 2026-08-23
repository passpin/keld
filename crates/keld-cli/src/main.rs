fn main() {
    std::process::exit(keld_cli::run_cli(std::env::args_os().skip(1)));
}
