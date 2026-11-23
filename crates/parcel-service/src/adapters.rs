use std::collections::HashSet;

use windows_sys::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows_sys::core::GUID;

/// GUIDs (`{XXXXXXXX-...}`) of the hardware network adapters that are up. Npcap device names end
/// with them; virtual adapters (Hyper-V, VPN, loopback) are left out.
///
/// # Errors
/// The Win32 error code when the interface table can't be read.
pub fn hardware_up() -> Result<HashSet<String>, u32> {
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    // SAFETY: on success `table` points to a table we free below.
    let err = unsafe { GetIfTable2(&raw mut table) };
    if err != 0 {
        return Err(err);
    }
    // SAFETY: GetIfTable2 succeeded, so `table` holds `NumEntries` rows.
    let rows = unsafe {
        std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize)
    };
    let guids = rows
        .iter()
        .filter(|row| {
            let hardware = row.InterfaceAndOperStatusFlags._bitfield & 1 != 0;
            hardware && row.OperStatus == IfOperStatusUp
        })
        .map(|row| guid_string(&row.InterfaceGuid))
        .collect();
    // SAFETY: `table` came from GetIfTable2 and is not used after this.
    unsafe { FreeMibTable(table.cast()) };
    Ok(guids)
}

fn guid_string(g: &GUID) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g.data1,
        g.data2,
        g.data3,
        g.data4[0],
        g.data4[1],
        g.data4[2],
        g.data4[3],
        g.data4[4],
        g.data4[5],
        g.data4[6],
        g.data4[7]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_formats_like_npcap_device_names() {
        let g = GUID {
            data1: 0x1234_ABCD,
            data2: 0x0A0B,
            data3: 0xC0DE,
            data4: [1, 2, 3, 4, 5, 6, 7, 0xFF],
        };
        assert_eq!(guid_string(&g), "{1234ABCD-0A0B-C0DE-0102-0304050607FF}");
    }
}
