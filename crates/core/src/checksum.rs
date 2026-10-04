//! SHA-256 of image files, and finding the checksum a distro published next to them.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::{check_cancel, Result};

/// Hashes the file as it is on disk (compressed files included: that's what
/// distros publish). `on_progress(bytes_done, total)` is throttled.
pub fn sha256_file(
    path: &Path,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<String> {
    let mut file = File::open(path)?;
    let total = file.metadata()?.len();
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    let mut last = Instant::now();
    loop {
        check_cancel(cancel)?;
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        done += n as u64;
        if last.elapsed() >= Duration::from_millis(150) {
            last = Instant::now();
            on_progress(done, total);
        }
    }
    on_progress(total, total);
    Ok(hex::encode(hasher.finalize()))
}

/// Normalises a pasted or parsed hash: lowercase hex, 64 characters.
pub fn normalize(hash: &str) -> Option<String> {
    let h = hash.trim().to_ascii_lowercase();
    (h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())).then_some(h)
}

/// Finds `file_name`'s SHA-256 in the text of a checksum file. Understands
/// GNU (`<hash>  <name>` / `<hash> *<name>`), BSD (`SHA256 (<name>) = <hash>`)
/// and a bare hash when the file is dedicated to one image.
pub fn find_in_sums(text: &str, file_name: &str, dedicated: bool) -> Option<String> {
    let mut lone = None;
    let mut lines = 0;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with("-----") {
            continue;
        }
        lines += 1;
        if let Some(rest) = line.strip_prefix("SHA256 (") {
            if let Some((name, hash)) = rest.split_once(") = ") {
                if name == file_name {
                    return normalize(hash);
                }
            }
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let (Some(hash), name) = (parts.next(), parts.next()) else {
            continue;
        };
        let Some(hash) = normalize(hash) else {
            continue;
        };
        match name.map(|n| {
            n.trim()
                .trim_start_matches(['*', ' '])
                .trim_start_matches("./")
        }) {
            Some(n) if n == file_name => return Some(hash),
            None => lone = Some(hash),
            Some(_) => {}
        }
    }
    // "ubuntu.iso.sha256" containing just the hash.
    if dedicated && lines == 1 {
        return lone;
    }
    None
}

/// Checksum files worth looking at next to an image, most specific first.
fn candidates(image: &Path) -> Vec<(PathBuf, bool)> {
    let Some(dir) = image.parent() else {
        return Vec::new();
    };
    let name = image
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut out: Vec<(PathBuf, bool)> = ["sha256", "sha256sum", "sha256.txt", "SHA256"]
        .iter()
        .map(|ext| (dir.join(format!("{name}.{ext}")), true))
        .collect();
    for shared in [
        "SHA256SUMS",
        "SHA256SUMS.txt",
        "sha256sum.txt",
        "sha256sums.txt",
        "CHECKSUM",
    ] {
        out.push((dir.join(shared), false));
    }
    // Fedora: Fedora-Workstation-41-1.4-x86_64-CHECKSUM
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.ends_with("-CHECKSUM") || n.ends_with(".CHECKSUM") {
                out.push((e.path(), false));
            }
        }
    }
    out
}

/// A checksum published alongside `image`, with the name of the file it came from.
pub fn find_published(image: &Path) -> Option<(String, String)> {
    let file_name = image.file_name()?.to_string_lossy().into_owned();
    for (path, dedicated) in candidates(image) {
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        // Checksum files are tiny; don't slurp something huge by mistake.
        if !meta.is_file() || meta.len() > 1 << 20 {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(hash) = find_in_sums(&text, &file_name, dedicated) {
            let source = path.file_name()?.to_string_lossy().into_owned();
            return Some((hash, source));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const H: &str = "c2e6f4dc9b0e2f7a51d3c8e4b6a90f12d7e5c3b1a8f6e4d2c0b9a7f5e3d1a1f3";
    const H2: &str = "0000000000000000000000000000000000000000000000000000000000000001";

    #[test]
    fn parses_gnu_bsd_and_bare() {
        let gnu = format!("{H2}  other.iso\n{H} *ubuntu-24.04.1-desktop-amd64.iso\n");
        assert_eq!(
            find_in_sums(&gnu, "ubuntu-24.04.1-desktop-amd64.iso", false).as_deref(),
            Some(H)
        );
        let bsd = format!(
            "-----BEGIN PGP SIGNED MESSAGE-----\n# Fedora\nSHA256 (Fedora.iso) = {}\n",
            H.to_uppercase()
        );
        assert_eq!(find_in_sums(&bsd, "Fedora.iso", false).as_deref(), Some(H));
        assert_eq!(
            find_in_sums(&format!("{H}\n"), "x.iso", true).as_deref(),
            Some(H)
        );
        assert_eq!(find_in_sums(&format!("{H}\n"), "x.iso", false), None);
        assert_eq!(find_in_sums(&gnu, "missing.iso", false), None);
        assert_eq!(find_in_sums("not a hash  x.iso", "x.iso", false), None);
    }

    #[test]
    fn hashes_and_finds_sums_next_to_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("disk.img");
        File::create(&img).unwrap().write_all(b"hello").unwrap();
        let actual = sha256_file(&img, &AtomicBool::new(false), &mut |_, _| {}).unwrap();
        assert_eq!(
            actual,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );

        assert_eq!(find_published(&img), None);
        std::fs::write(
            dir.path().join("SHA256SUMS"),
            format!("{actual}  disk.img\n"),
        )
        .unwrap();
        assert_eq!(
            find_published(&img),
            Some((actual.clone(), "SHA256SUMS".into()))
        );
        // A dedicated file wins over the shared one.
        std::fs::write(dir.path().join("disk.img.sha256"), format!("{H}\n")).unwrap();
        assert_eq!(
            find_published(&img),
            Some((H.into(), "disk.img.sha256".into()))
        );
    }

    #[test]
    fn normalizes() {
        assert_eq!(
            normalize(&format!("  {}  ", H.to_uppercase())).as_deref(),
            Some(H)
        );
        assert_eq!(normalize("abc"), None);
    }
}
