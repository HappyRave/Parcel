use std::ffi::c_void;
use std::path::Path;

use parcel_core::process::{App, Processes};
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

/// Names processes by their executable: path for identity, the file's description (what Task
/// Manager shows) for display, else the file name.
pub struct WinProcesses {
    own_pid: u32,
}

impl WinProcesses {
    pub fn new() -> Self {
        Self {
            own_pid: std::process::id(),
        }
    }
}

impl Processes for WinProcesses {
    fn app(&mut self, pid: u32, image: Option<&str>) -> App {
        if pid == 0 || pid == 4 {
            return App {
                path: "System".to_owned(),
                name: "System".to_owned(),
            };
        }
        // The running process first; the start event's image once it has exited.
        let Some(path) = image_path(pid).or_else(|| image.map(str::to_owned)) else {
            return App {
                path: format!("pid:{pid}"),
                name: format!("Process {pid}"),
            };
        };
        let name = if pid == self.own_pid {
            "Parcel".to_owned()
        } else {
            description(&path).unwrap_or_else(|| file_stem(&path))
        };
        App { path, name }
    }
}

fn file_stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map_or_else(|| path.to_owned(), |s| s.to_string_lossy().into_owned())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// `None` when the process is gone or protected.
fn image_path(pid: u32) -> Option<String> {
    // SAFETY: plain query; the handle is closed below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `buf` holds `len` u16s.
    let ok = unsafe {
        QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &raw mut len)
    };
    // SAFETY: `handle` came from OpenProcess.
    unsafe { CloseHandle(handle) };
    (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
}

/// The `FileDescription` from the executable's version resource.
fn description(path: &str) -> Option<String> {
    let path = wide(path);
    // SAFETY: `path` is NUL-terminated.
    let size = unsafe { GetFileVersionInfoSizeW(path.as_ptr(), std::ptr::null_mut()) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    // SAFETY: `data` holds `size` bytes.
    if unsafe { GetFileVersionInfoW(path.as_ptr(), 0, size, data.as_mut_ptr().cast()) } == 0 {
        return None;
    }
    let mut blocks = Vec::new();
    if let Some(t) = query(&data, r"\VarFileInfo\Translation")
        && t.len() >= 4
    {
        let lang = u16::from_le_bytes([t[0], t[1]]);
        let page = u16::from_le_bytes([t[2], t[3]]);
        blocks.push(format!("{lang:04x}{page:04x}"));
    }
    blocks.extend(["040904b0".to_owned(), "040904e4".to_owned()]);
    blocks.iter().find_map(|block| {
        let raw = query(&data, &format!(r"\StringFileInfo\{block}\FileDescription"))?;
        let chars: Vec<u16> = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| u16::from_le_bytes(c))
            .take_while(|&c| c != 0)
            .collect();
        let text = String::from_utf16_lossy(&chars).trim().to_owned();
        (!text.is_empty()).then_some(text)
    })
}

/// A value inside version-info `data`, as bytes (`VerQueryValueW` counts strings in u16s).
fn query<'a>(data: &'a [u8], sub_block: &str) -> Option<&'a [u8]> {
    let sub = wide(sub_block);
    let mut ptr: *mut c_void = std::ptr::null_mut();
    let mut len = 0u32;
    // SAFETY: `data` is a version-info block; `ptr` will point inside it.
    let ok = unsafe {
        VerQueryValueW(
            data.as_ptr().cast(),
            sub.as_ptr(),
            &raw mut ptr,
            &raw mut len,
        )
    };
    if ok == 0 || ptr.is_null() || len == 0 {
        return None;
    }
    let bytes = if sub_block.starts_with(r"\StringFileInfo") {
        len as usize * 2
    } else {
        len as usize
    };
    // The pointer lands inside `data`; slicing safely keeps a bad length from reading past it.
    let start = (ptr as usize).checked_sub(data.as_ptr() as usize)?;
    data.get(start..(start + bytes).min(data.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_process_is_parcel() {
        let app = WinProcesses::new().app(std::process::id(), None);
        assert_eq!(app.name, "Parcel");
        assert!(app.path.ends_with(".exe"), "{}", app.path);
    }

    #[test]
    fn system_pids_are_system() {
        assert_eq!(WinProcesses::new().app(4, None).name, "System");
    }

    #[test]
    fn gone_process_gets_a_stand_in() {
        let app = WinProcesses::new().app(u32::MAX - 3, None);
        assert_eq!(app.name, format!("Process {}", u32::MAX - 3));
    }

    #[test]
    fn gone_process_is_named_from_its_image() {
        let root = std::env::var("SystemRoot").unwrap();
        let cmd = format!(r"{root}\System32\cmd.exe");
        let app = WinProcesses::new().app(u32::MAX - 3, Some(&cmd));
        assert_eq!(
            (app.path, app.name),
            (cmd, "Windows Command Processor".to_owned())
        );
    }

    #[test]
    fn a_live_process_wins_over_a_stale_image() {
        let app = WinProcesses::new().app(std::process::id(), Some(r"C:\old\other.exe"));
        assert_ne!(app.path, r"C:\old\other.exe");
    }

    #[test]
    fn windows_binaries_have_descriptions() {
        let root = std::env::var("SystemRoot").unwrap();
        let cmd = format!(r"{root}\System32\cmd.exe");
        assert_eq!(
            description(&cmd).as_deref(),
            Some("Windows Command Processor")
        );
    }
}
