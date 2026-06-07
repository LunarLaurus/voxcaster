//! Self-bundles the Windows ConPTY redistributable so `portable-pty` uses a
//! modern ConPTY host instead of the (sometimes broken) inbox one (e.g. Win10
//! LTSC 19044 / IoT).
//!
//! On non-Windows this module compiles to a single no-op function.

/// Ensure a modern `conpty.dll` + `OpenConsole.exe` are available to
/// `portable-pty`.  No-op on non-Windows.  Best-effort: logs a warning to
/// stderr on failure but never panics — `portable-pty` falls back to the inbox
/// ConPTY if the staging step fails.
pub fn ensure_conpty() {
    #[cfg(windows)]
    windows_impl::ensure();
}

#[cfg(windows)]
mod windows_impl {
    use std::path::{Path, PathBuf};

    const CONPTY_DLL: &[u8] = include_bytes!("../assets/win-x64/conpty.dll");
    const OPENCONSOLE_EXE: &[u8] = include_bytes!("../assets/win-x64/OpenConsole.exe");

    pub(super) fn ensure() {
        // 1. Pick a writable extract dir: %LOCALAPPDATA%\voxcaster\conpty
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let dir = base.join("voxcaster").join("conpty");

        if let Err(e) = extract_to(&dir) {
            eprintln!(
                "voxcaster: warning: could not stage ConPTY redistributable: {e}; \
                 falling back to system ConPTY"
            );
            return;
        }

        // 2. Add the dir to the DLL search path so LoadLibrary("conpty.dll")
        //    finds ours before the (possibly broken) inbox one.
        add_dll_directory(&dir);
    }

    pub(super) fn extract_to(dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        write_if_changed(&dir.join("conpty.dll"), CONPTY_DLL)?;
        write_if_changed(&dir.join("OpenConsole.exe"), OPENCONSOLE_EXE)?;
        Ok(())
    }

    pub(super) fn write_if_changed(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        // Avoid rewriting (and locking issues) if the file is already current.
        if let Ok(existing) = std::fs::read(path) {
            if existing.len() == bytes.len() && existing == bytes {
                return Ok(());
            }
        }
        std::fs::write(path, bytes)
    }

    fn add_dll_directory(dir: &Path) {
        use std::os::windows::ffi::OsStrExt;

        let wide: Vec<u16> = dir
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // SetDllDirectoryW inserts `dir` into the search order immediately after
        // the executable's own directory and before the system directory, so our
        // modern conpty.dll wins over the inbox one.  OpenConsole.exe sits beside
        // it and is found by conpty.dll itself.
        #[link(name = "kernel32")]
        extern "system" {
            fn SetDllDirectoryW(lpPathName: *const u16) -> i32;
        }

        // SAFETY: `wide` is a valid, null-terminated UTF-16 path.
        unsafe {
            SetDllDirectoryW(wide.as_ptr());
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::time::{SystemTime, UNIX_EPOCH};

        fn unique_test_dir() -> std::path::PathBuf {
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos();
            std::env::temp_dir().join(format!("voxcaster-test-{ts}"))
        }

        #[test]
        fn test_extract_creates_both_files() {
            let dir = unique_test_dir();
            extract_to(&dir).expect("extract_to should succeed");

            let dll = dir.join("conpty.dll");
            let exe = dir.join("OpenConsole.exe");

            assert!(dll.exists(), "conpty.dll should exist after extraction");
            assert!(
                exe.exists(),
                "OpenConsole.exe should exist after extraction"
            );

            // Verify sizes match the embedded bytes.
            let dll_len = std::fs::metadata(&dll).unwrap().len() as usize;
            let exe_len = std::fs::metadata(&exe).unwrap().len() as usize;
            assert_eq!(dll_len, CONPTY_DLL.len(), "conpty.dll length mismatch");
            assert_eq!(
                exe_len,
                OPENCONSOLE_EXE.len(),
                "OpenConsole.exe length mismatch"
            );

            // Clean up.
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn test_extract_is_idempotent() {
            let dir = unique_test_dir();

            // First extraction.
            extract_to(&dir).expect("first extract_to should succeed");

            // Record modification times.
            let dll_path = dir.join("conpty.dll");
            let exe_path = dir.join("OpenConsole.exe");
            let dll_mtime_1 = std::fs::metadata(&dll_path).unwrap().modified().unwrap();
            let exe_mtime_1 = std::fs::metadata(&exe_path).unwrap().modified().unwrap();

            // Brief pause so a re-write would produce a detectably different mtime.
            std::thread::sleep(std::time::Duration::from_millis(10));

            // Second extraction — should be a no-op (write_if_changed skips equal content).
            extract_to(&dir).expect("second extract_to should succeed");

            let dll_mtime_2 = std::fs::metadata(&dll_path).unwrap().modified().unwrap();
            let exe_mtime_2 = std::fs::metadata(&exe_path).unwrap().modified().unwrap();

            assert_eq!(
                dll_mtime_1, dll_mtime_2,
                "conpty.dll should not be rewritten"
            );
            assert_eq!(
                exe_mtime_1, exe_mtime_2,
                "OpenConsole.exe should not be rewritten"
            );

            // Clean up.
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
