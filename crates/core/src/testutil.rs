//! Shared test helpers.

use std::path::Path;
use std::process::Command;

/// Builds a UDF image of `src` with whatever tool this machine has.
/// Returns false if none is available, so the caller can skip.
pub fn make_udf(src: &Path, out: &Path) -> bool {
    let tries: [(&str, &[&str]); 3] = [
        (
            "hdiutil",
            &["makehybrid", "-udf", "-udf-version", "1.02", "-o"],
        ),
        ("genisoimage", &["-quiet", "-udf", "-o"]),
        ("mkisofs", &["-quiet", "-udf", "-o"]),
    ];
    tries.iter().any(|(tool, args)| {
        Command::new(tool)
            .args(*args)
            .arg(out)
            .arg(src)
            .output()
            .is_ok_and(|o| o.status.success())
    })
}
