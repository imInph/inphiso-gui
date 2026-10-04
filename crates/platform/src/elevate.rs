//! Running the helper with administrator rights: an admin password prompt on
//! macOS, pkexec on Linux and UAC on Windows.

use std::path::Path;

use crate::{Error, Result};

/// Set to `1` to run the helper without elevation (development, or when already root).
pub const NO_ELEVATE_ENV: &str = "INPHISO_NO_ELEVATE";

/// Shown in the macOS password dialog. Linux takes its message from the polkit
/// policy and Windows UAC shows the program name.
#[cfg(target_os = "macos")]
const PROMPT: &str = "inphiso needs your permission to write to the drive.";

/// Runs `program` elevated and waits for it to exit. Returns
/// [`Error::AuthCancelled`] if the user dismissed the prompt.
pub fn run_elevated(program: &Path, args: &[String]) -> Result<()> {
    if std::env::var(NO_ELEVATE_ENV).as_deref() == Ok("1") || is_root() {
        return run_direct(program, args);
    }
    os_run_elevated(program, args)
}

/// Runs `program` as the current user and waits for it.
pub fn run_direct(program: &Path, args: &[String]) -> Result<()> {
    let status = std::process::Command::new(program)
        .args(args)
        .status()
        .map_err(|source| Error::Spawn {
            cmd: "inphiso-helper",
            source,
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Command {
            cmd: "inphiso-helper",
            stderr: format!("exited with {status}"),
        })
    }
}

#[cfg(unix)]
fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[cfg(windows)]
fn is_root() -> bool {
    false
}

/// Single-quotes for /bin/sh.
#[allow(dead_code)]
pub(crate) fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Escapes for the inside of an AppleScript string literal.
#[allow(dead_code)]
pub(crate) fn applescript_escape(s: &str) -> String {
    s.replace('\\', r"\\").replace('"', "\\\"")
}

/// Quotes one argument the way CommandLineToArgvW will split it back.
#[allow(dead_code)]
pub(crate) fn win_quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '"']) {
        return arg.to_string();
    }
    let mut out = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

#[cfg(target_os = "macos")]
fn os_run_elevated(program: &Path, args: &[String]) -> Result<()> {
    let mut cmd = sh_quote(&program.to_string_lossy());
    for a in args {
        cmd.push(' ');
        cmd.push_str(&sh_quote(a));
    }
    // Keep the helper's output off the pipe; `do shell script` buffers it all until exit.
    cmd.push_str(" >/dev/null 2>&1");
    let script = format!(
        "do shell script \"{}\" with administrator privileges with prompt \"{}\"",
        applescript_escape(&cmd),
        applescript_escape(PROMPT),
    );
    let out = std::process::Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|source| Error::Spawn {
            cmd: "osascript",
            source,
        })?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("(-128)") {
        return Err(Error::AuthCancelled);
    }
    Err(Error::Command {
        cmd: "inphiso-helper",
        stderr: stderr.trim().to_string(),
    })
}

#[cfg(target_os = "linux")]
fn os_run_elevated(program: &Path, args: &[String]) -> Result<()> {
    // Inside an AppImage the helper lives on a FUSE mount that root can't read.
    // Copy it somewhere private first.
    let copied;
    let program = if std::env::var_os("APPIMAGE").is_some() {
        copied = copy_out_of_appimage(program)?;
        copied.as_path()
    } else {
        program
    };
    let status = std::process::Command::new("pkexec")
        .arg(program)
        .args(args)
        .status()
        .map_err(|source| Error::Spawn {
            cmd: "pkexec",
            source,
        })?;
    match status.code() {
        Some(0) => Ok(()),
        // 126: the user dismissed the dialog. 127: not authorized.
        Some(126) => Err(Error::AuthCancelled),
        Some(127) => Err(Error::Refused(
            "You aren't allowed to run inphiso's writer as administrator.".into(),
        )),
        _ => Err(Error::Command {
            cmd: "inphiso-helper",
            stderr: format!("exited with {status}"),
        }),
    }
}

#[cfg(target_os = "linux")]
fn copy_out_of_appimage(program: &Path) -> Result<std::path::PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("inphiso-{}", crate::channel::random_hex(8)));
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    let dest = dir.join("inphiso-helper");
    std::fs::copy(program, &dest)?;
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o700))?;
    Ok(dest)
}

#[cfg(windows)]
fn os_run_elevated(program: &Path, args: &[String]) -> Result<()> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SEE_MASK_NO_CONSOLE, SHELLEXECUTEINFOW,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let file = HSTRING::from(program.as_os_str());
    let params = HSTRING::from(
        args.iter()
            .map(|a| win_quote(a))
            .collect::<Vec<_>>()
            .join(" "),
    );
    let verb = HSTRING::from("runas");
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NO_CONSOLE,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        if e.code() == ERROR_CANCELLED.to_hresult() {
            return Err(Error::AuthCancelled);
        }
        return Err(Error::Command {
            cmd: "inphiso-helper",
            stderr: e.message(),
        });
    }
    let mut code = 0u32;
    unsafe {
        WaitForSingleObject(info.hProcess, INFINITE);
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
    }
    if code == 0 {
        Ok(())
    } else {
        Err(Error::Command {
            cmd: "inphiso-helper",
            stderr: format!("exited with code {code}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(
            sh_quote("/Applications/My App/x"),
            "'/Applications/My App/x'"
        );
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
        assert_eq!(applescript_escape(r#"a "b" \c"#), r#"a \"b\" \\c"#);
        assert_eq!(win_quote("plain"), "plain");
        assert_eq!(win_quote(r"C:\Program Files\x"), r#""C:\Program Files\x""#);
        assert_eq!(win_quote(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(win_quote(r"ends with\ "), r#""ends with\ ""#);
        assert_eq!(win_quote(r"dir\"), r"dir\");
        assert_eq!(win_quote(r"a b\"), r#""a b\\""#);
        assert_eq!(win_quote(""), r#""""#);
    }
}
