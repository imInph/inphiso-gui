//! `inphiso-helper` is the only part of inphiso that runs elevated. The GUI
//! launches it through pkexec / an admin prompt / UAC, it connects back over a
//! local socket, runs exactly one flash job, and exits.

fn main() -> anyhow::Result<()> {
    eprintln!("inphiso-helper {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
