//! The game process: launch it through Steam, tell whether it's running, open Workshop pages.

use std::io;

pub const STEAM_APP_ID: u32 = 294_100;
pub const EXE: &str = "RimWorld.exe";

/// Starts RimWorld through Steam (same as pressing Play in Steam).
pub fn launch() -> io::Result<()> {
    open(&format!("steam://rungameid/{STEAM_APP_ID}"))
}

pub fn workshop_url(steam_id: u64) -> String {
    format!("steam://url/CommunityFilePage/{steam_id}")
}

/// Hands a URL (steam://, https://) or a folder to the system shell.
pub fn open(target: &str) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer").arg(target).spawn().map(|_| ())
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(target).spawn().map(|_| ())
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        Err(io::Error::new(io::ErrorKind::Other, "Unsupported platform"))
    }
}

/// True if a process with this executable name is running.
pub fn is_running(exe: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
        };
        // SAFETY: plain Win32 calls on a zeroed, correctly sized struct; the snapshot handle is always closed.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return false;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut found = false;
            let mut more = Process32FirstW(snapshot, &mut entry) != 0;
            while more && !found {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                found = String::from_utf16_lossy(&entry.szExeFile[..len]).eq_ignore_ascii_case(exe);
                more = Process32NextW(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
            found
        }
    }

    #[cfg(target_os = "linux")]
    {
        // En Linux, exploramos /proc para buscar procesos activos (incluyendo Wine/Proton)
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return false;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                if file_name.chars().all(|c| c.is_ascii_digit()) {
                    // Comprobar por comm (nombre del binario)
                    let comm_path = path.join("comm");
                    if let Ok(comm) = std::fs::read_to_string(comm_path) {
                        if comm.trim().eq_ignore_ascii_case(exe) {
                            return true;
                        }
                    }
                    // Comprobar por cmdline (por si corre bajo Wine/Proton ejecutando el .exe)
                    let cmdline_path = path.join("cmdline");
                    if let Ok(cmdline) = std::fs::read_to_string(cmdline_path) {
                        if cmdline.split('\0').any(|arg| arg.to_lowercase().ends_with(&exe.to_lowercase())) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_running_processes() {
        let me = std::env::current_exe().unwrap();
        let me_name = me.file_name().unwrap().to_str().unwrap();
        assert!(is_running(me_name), "the test binary itself is running (case-insensitive)");
        assert!(!is_running("surely-not-a-real-process-1234.exe"));
    }

    #[test]
    fn workshop_urls() {
        assert_eq!(workshop_url(2009463077), "steam://url/CommunityFilePage/2009463077");
    }
}
