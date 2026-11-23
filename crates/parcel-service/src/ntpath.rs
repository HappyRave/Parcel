use windows_sys::Win32::Storage::FileSystem::QueryDosDeviceW;

/// Turns kernel paths (`\Device\HarddiskVolume3\Windows\notepad.exe`), as ETW reports process
/// images, into drive paths (`C:\Windows\notepad.exe`), the form the rest of Parcel uses.
pub struct DeviceMap {
    /// (device, drive), e.g. (`\Device\HarddiskVolume3`, `C:`).
    drives: Vec<(String, String)>,
}

impl DeviceMap {
    /// Reads the current drive letters. Drives mounted later aren't known.
    pub fn current() -> Self {
        let mut drives = Vec::new();
        for letter in b'A'..=b'Z' {
            let drive = format!("{}:", char::from(letter));
            let name: Vec<u16> = drive.encode_utf16().chain(Some(0)).collect();
            let mut target = [0u16; 1024];
            // SAFETY: `name` is NUL-terminated and `target` holds the length passed.
            let len =
                unsafe { QueryDosDeviceW(name.as_ptr(), target.as_mut_ptr(), target.len() as u32) };
            if len == 0 {
                continue;
            }
            // The result is a list of NUL-terminated strings; the first is the device.
            let device: Vec<u16> = target.iter().copied().take_while(|&c| c != 0).collect();
            drives.push((String::from_utf16_lossy(&device), drive));
        }
        Self { drives }
    }

    /// The drive path for `nt`; `None` when it isn't on a known drive.
    pub fn to_dos(&self, nt: &str) -> Option<String> {
        self.drives.iter().find_map(|(device, drive)| {
            let rest = nt.strip_prefix(device.as_str())?;
            // `\Device\HarddiskVolume1` must not match `\Device\HarddiskVolume10\...`.
            rest.starts_with('\\').then(|| format!("{drive}{rest}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> DeviceMap {
        DeviceMap {
            drives: vec![
                (r"\Device\HarddiskVolume1".into(), "D:".into()),
                (r"\Device\HarddiskVolume10".into(), "E:".into()),
                (r"\Device\HarddiskVolume3".into(), "C:".into()),
            ],
        }
    }

    #[test]
    fn device_paths_become_drive_paths() {
        assert_eq!(
            map()
                .to_dos(r"\Device\HarddiskVolume3\Windows\System32\curl.exe")
                .as_deref(),
            Some(r"C:\Windows\System32\curl.exe")
        );
        assert_eq!(
            map()
                .to_dos(r"\Device\HarddiskVolume10\Games\x.exe")
                .as_deref(),
            Some(r"E:\Games\x.exe")
        );
    }

    #[test]
    fn unknown_devices_stay_unmapped() {
        assert_eq!(map().to_dos(r"\Device\Mup\server\share\x.exe"), None);
    }

    #[test]
    fn the_system_drive_is_mapped_live() {
        let windir = std::env::var("SystemRoot").unwrap();
        let drive = &windir[..2];
        let live = DeviceMap::current();
        assert!(
            live.drives
                .iter()
                .any(|(_, d)| d.eq_ignore_ascii_case(drive)),
            "{drive} missing"
        );
    }
}
