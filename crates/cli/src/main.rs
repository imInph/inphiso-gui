use anyhow::Result;

const USAGE: &str = "\
usage: inphiso <command>

commands:
  list-drives [--all] [--json]   removable drives (--all: every drive except the system disk)
  version                        print the version";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    match args.first().map(String::as_str) {
        Some("list-drives") => list_drives(flag("--all"), flag("--json")),
        Some("version") => {
            println!("inphiso {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

fn list_drives(all: bool, json: bool) -> Result<()> {
    let drives = inphiso_platform::list_drives(all)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&drives)?);
        return Ok(());
    }
    if drives.is_empty() {
        println!("no drives found");
    }
    for d in drives {
        println!(
            "{:<22} {:>9}  {:<8} {}{}",
            d.id,
            format!("{:.1} GB", d.size as f64 / 1e9),
            d.bus.as_deref().unwrap_or("-"),
            d.name,
            if d.volumes.is_empty() {
                String::new()
            } else {
                format!("  [{}]", d.volumes.join(", "))
            },
        );
    }
    Ok(())
}
