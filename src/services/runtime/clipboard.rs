#[cfg(target_os = "linux")]
use std::io::ErrorKind;
use std::io::Write;
use std::process::{Command, Stdio};

pub trait ClipboardWriter: Send + Sync {
    fn set_text(&self, value: &str) -> Result<(), String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PlatformClipboardWriter;

impl ClipboardWriter for PlatformClipboardWriter {
    fn set_text(&self, value: &str) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            run_clipboard_command("pbcopy", &[], value)
        }

        #[cfg(target_os = "windows")]
        {
            run_clipboard_command("clip", &[], value)
        }

        #[cfg(target_os = "linux")]
        {
            for (program, args) in [
                ("wl-copy", &[][..]),
                ("xclip", &["-selection", "clipboard"][..]),
                ("xsel", &["--clipboard", "--input"][..]),
            ] {
                if try_run_clipboard_command(program, args, value)? {
                    return Ok(());
                }
            }

            Err("no supported clipboard command found (tried wl-copy, xclip, xsel)".to_string())
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            let _ = value;
            Err("clipboard is unsupported on this platform".to_string())
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn run_clipboard_command(program: &str, args: &[&str], value: &str) -> Result<(), String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{program} is unavailable: {error}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| format!("{program} did not accept clipboard input"))?;
    stdin
        .write_all(value.as_bytes())
        .map_err(|error| format!("failed to write clipboard input to {program}: {error}"))?;
    drop(stdin);
    let status = child
        .wait()
        .map_err(|error| format!("failed to wait for {program}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with status {status}"))
    }
}

#[cfg(target_os = "linux")]
fn try_run_clipboard_command(program: &str, args: &[&str], value: &str) -> Result<bool, String> {
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("failed to start {program}: {error}")),
    };
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| format!("{program} did not accept clipboard input"))?;
    stdin
        .write_all(value.as_bytes())
        .map_err(|error| format!("failed to write clipboard input to {program}: {error}"))?;
    drop(stdin);
    let status = child
        .wait()
        .map_err(|error| format!("failed to wait for {program}: {error}"))?;
    if status.success() {
        Ok(true)
    } else {
        Err(format!("{program} exited with status {status}"))
    }
}
