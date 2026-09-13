fn main() {
    // Per-file panics are caught during the run; keep their default messages off stderr.
    std::panic::set_hook(Box::new(|_| {}));

    match oa_sample_analyzer::args::Config::parse(std::env::args().collect()) {
        Some(cfg) => oa_sample_analyzer::run::run(&cfg),
        None => {
            eprintln!("usage: oa_sample_analyzer <dir> [--out <path>] [--workers <n>] [--max-len <s>] [--clusters <k>] [--no-per-file] [--force]");
            std::process::exit(2);
        }
    }
}
