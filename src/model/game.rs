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

/// Hands a URL (steam://, https://) or a folder to the Windows shell.
pub fn open(target: &str) -> io::Result<()> {
    std::process::Command::new("explorer").arg(target).spawn().map(|_| ())
}

/// True if a process with this executable name is running. Native process snapshot: no child process, ~1 ms.
pub fn is_running(exe: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_running_processes() {
        let me = std::env::current_exe().unwrap();
        let me = me.file_name().unwrap().to_str().unwrap().to_uppercase();
        assert!(is_running(&me), "the test binary itself is running (case-insensitive)");
        assert!(!is_running("surely-not-a-real-process-1234.exe"));
    }

    #[test]
    fn workshop_urls() {
        assert_eq!(workshop_url(2009463077), "steam://url/CommunityFilePage/2009463077");
    }
}
