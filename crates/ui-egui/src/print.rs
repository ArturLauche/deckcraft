//! Native printing where the platform offers a command-line spooler.
//!
//! DeckCraft's shared print path is a PDF handed to the system viewer, which works everywhere.
//! On Unix-like systems with CUPS, “Send directly to printer” additionally runs `lp`; if it is
//! missing or fails, the caller falls back to the PDF viewer. The path is passed as an argument,
//! never through a shell.

use std::process::Command;

/// Send `path` to the default printer with `lp`. Errors are returned so the caller can open the
/// PDF instead.
pub fn send_to_printer(path: &str) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        run_printer("lp", path)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        Err("direct printing is not available in the browser".into())
    }
}

/// Run `program path` and require a successful exit. Kept separate so tests can use a fake
/// spooler without changing the process environment.
#[cfg(not(target_arch = "wasm32"))]
fn run_printer(program: &str, path: &str) -> Result<(), String> {
    let status = Command::new(program).arg(path).status().map_err(|e| format!("couldn't run {program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with {}", status.code().map_or_else(|| "a signal".to_string(), |c| c.to_string())))
    }
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_fake_spooler_receives_the_pdf_path() {
        let dir = std::env::temp_dir().join(format!("deckcraft-print-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("lp");
        std::fs::write(&script, "#!/bin/sh\nprintf '%s' \"$1\" > \"$1.spooled\"\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let pdf = dir.join("deck.pdf");
        std::fs::write(&pdf, b"%PDF").unwrap();
        super::run_printer(script.to_str().unwrap(), pdf.to_str().unwrap()).unwrap();
        assert_eq!(std::fs::read_to_string(format!("{}.spooled", pdf.display())).unwrap(), pdf.to_str().unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_spooler_is_an_error() {
        let err = super::run_printer("deckcraft-no-such-printer", "/tmp/none.pdf").unwrap_err();
        assert!(err.contains("couldn't run"), "{err}");
    }
}
