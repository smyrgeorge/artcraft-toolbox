#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

fn main() {
    // `args_os`, not `args`: a non-Unicode argument must be a usage error, not a panic.
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let code = artcraft_toolbox_cli::run_os(&args, &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}
