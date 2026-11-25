//! CLI wrapper around [`newpal_prepare::hd_batch`].
fn main() {
    if let Err(e) = newpal_prepare::hd_batch::run_from_env() {
        eprintln!("error: {e}");
        std::process::exit(2);
    }
}
