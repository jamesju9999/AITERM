use aiterm_core::pty::elevated_protocol as protocol;

#[cfg(windows)]
mod windows_host;

fn main() {
    #[cfg(windows)]
    {
        windows_host::run();
    }
    #[cfg(not(windows))]
    {
        eprintln!("aiterm-elevated-host only runs on Windows");
        std::process::exit(1);
    }
}
