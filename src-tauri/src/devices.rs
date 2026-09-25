//! Windows Raw Input wheel/gamepad discovery and SDL2 DirectInput-style GUID creation.

use crate::ini_edit::parse;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::mem::size_of;
use std::path::PathBuf;
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HidD_GetManufacturerString, HidD_GetProductString,
};
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::UI::Input::{
    GetRawInputDeviceInfoW, GetRawInputDeviceList, RAWINPUTDEVICELIST, RIDI_DEVICEINFO,
    RIDI_DEVICENAME, RID_DEVICE_INFO, RIM_TYPEHID,
};

const ERROR_RESULT: u32 = u32::MAX;

#[derive(Debug, Clone, Serialize)]
pub struct DeviceEntry {
    name: String,
    product_string: String,
    vendor_id: u16,
    product_id: u16,
    version: u16,
    usage: u16,
    derived_guid: String,
    guid: String,
    note: String,
    kind: String,
}

#[derive(Debug)]
struct RawDevice {
    manufacturer: String,
    product: String,
    vendor: u16,
    product_id: u16,
    version: u16,
    usage: u16,
}

fn crc16(initial: u16, data: &[u8]) -> u16 {
    let mut crc = initial;
    for byte in data {
        let mut r = (crc as u8) ^ byte;
        let mut byte_crc = 0_u16;
        for _ in 0..8 {
            byte_crc = (if ((byte_crc as u8) ^ r) & 1 != 0 {
                0xa001
            } else {
                0
            }) ^ (byte_crc >> 1);
            r >>= 1;
        }
        crc = byte_crc ^ (crc >> 8);
    }
    crc
}

fn sdl_guid(vendor: u16, product: u16, product_name: &str) -> String {
    let mut crc = 0;
    if !product_name.is_empty() {
        crc = crc16(crc, product_name.as_bytes());
    }

    // SDL's DirectInput backend passes the product name without a manufacturer,
    // uses version zero, and supplies no driver signature. That is the GUID
    // format FFB Blaster expects, even though Raw Input reports more metadata.
    let mut bytes = [0_u8; 16];
    bytes[0..2].copy_from_slice(&3_u16.to_le_bytes());
    bytes[2..4].copy_from_slice(&crc.to_le_bytes());
    bytes[4..6].copy_from_slice(&vendor.to_le_bytes());
    bytes[8..10].copy_from_slice(&product.to_le_bytes());
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn vendor_label(vendor: u16) -> Option<&'static str> {
    match vendor {
        0x044f => Some("Thrustmaster"),
        0x046d => Some("Logitech"),
        0x0eb7 => Some("Fanatec"),
        0x346e => Some("MOZA Racing"),
        0x045e => Some("Microsoft"),
        0x054c => Some("Sony"),
        0x28de => Some("Valve"),
        0x0079 | 0x11ff => Some("DragonRise"),
        _ => None,
    }
}

fn friendly_name(raw: &RawDevice) -> String {
    if !raw.product.is_empty() {
        if raw.manufacturer.is_empty()
            || raw
                .product
                .to_lowercase()
                .starts_with(&raw.manufacturer.to_lowercase())
        {
            return raw.product.clone();
        }
        return format!("{} {}", raw.manufacturer, raw.product);
    }
    vendor_label(raw.vendor)
        .map(|v| format!("{v} controller"))
        .unwrap_or_else(|| format!("HID controller {:04x}:{:04x}", raw.vendor, raw.product_id))
}

fn parse_guid(value: &str) -> Option<([u8; 16], u16, u16)> {
    let value = value.trim();
    if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut bytes = [0_u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    let vendor = u16::from_le_bytes([bytes[4], bytes[5]]);
    let product = u16::from_le_bytes([bytes[8], bytes[9]]);
    Some((bytes, vendor, product))
}

fn saved_guids(targets: &[String]) -> HashMap<(u16, u16), Vec<String>> {
    let mut found: HashMap<(u16, u16), Vec<String>> = HashMap::new();
    for target in targets {
        let Ok(content) = fs::read_to_string(PathBuf::from(target)) else {
            continue;
        };
        for entry in parse(&content) {
            if entry.section.eq_ignore_ascii_case("SETTINGS")
                && entry.key.eq_ignore_ascii_case("DeviceGUID")
            {
                if let Some((_, vendor, product)) = parse_guid(&entry.value) {
                    let values = found.entry((vendor, product)).or_default();
                    let normalized = entry.value.trim().to_ascii_lowercase();
                    if !values.contains(&normalized) {
                        values.push(normalized);
                    }
                }
            }
        }
    }
    found
}

unsafe fn hid_string(handle: windows_sys::Win32::Foundation::HANDLE, product: bool) -> String {
    let mut buffer = [0_u16; 256];
    let ok = if product {
        unsafe {
            HidD_GetProductString(
                handle,
                buffer.as_mut_ptr().cast(),
                size_of::<[u16; 256]>() as u32,
            )
        }
    } else {
        unsafe {
            HidD_GetManufacturerString(
                handle,
                buffer.as_mut_ptr().cast(),
                size_of::<[u16; 256]>() as u32,
            )
        }
    };
    if !ok {
        return String::new();
    }
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end]).trim().to_string()
}

unsafe fn query_raw_devices() -> Result<Vec<RawDevice>, String> {
    let list_size = size_of::<RAWINPUTDEVICELIST>() as u32;
    let mut count = 0_u32;
    if unsafe { GetRawInputDeviceList(std::ptr::null_mut(), &mut count, list_size) } == ERROR_RESULT
    {
        return Err(format!(
            "Windows could not list input devices: {}",
            std::io::Error::last_os_error()
        ));
    }
    if count == 0 {
        return Ok(Vec::new());
    }

    let mut devices = vec![RAWINPUTDEVICELIST::default(); count as usize];
    let returned = unsafe { GetRawInputDeviceList(devices.as_mut_ptr(), &mut count, list_size) };
    if returned == ERROR_RESULT {
        return Err(format!(
            "Windows could not read input devices: {}",
            std::io::Error::last_os_error()
        ));
    }

    let mut output = Vec::new();
    for item in devices.into_iter().take(returned as usize) {
        if item.dwType != RIM_TYPEHID {
            continue;
        }

        let mut info = RID_DEVICE_INFO::default();
        info.cbSize = size_of::<RID_DEVICE_INFO>() as u32;
        let mut info_size = info.cbSize;
        if unsafe {
            GetRawInputDeviceInfoW(
                item.hDevice,
                RIDI_DEVICEINFO,
                (&mut info as *mut RID_DEVICE_INFO).cast(),
                &mut info_size,
            )
        } == ERROR_RESULT
        {
            continue;
        }
        let hid = unsafe { info.Anonymous.hid };
        // Generic Desktop page: Joystick, Game Pad, or Multi-axis Controller.
        if hid.usUsagePage != 0x01 || !matches!(hid.usUsage, 0x04 | 0x05 | 0x08) {
            continue;
        }

        let mut chars = 0_u32;
        if unsafe {
            GetRawInputDeviceInfoW(
                item.hDevice,
                RIDI_DEVICENAME,
                std::ptr::null_mut(),
                &mut chars,
            )
        } == ERROR_RESULT
            || chars == 0
        {
            continue;
        }
        let mut path = vec![0_u16; chars as usize + 1];
        if unsafe {
            GetRawInputDeviceInfoW(
                item.hDevice,
                RIDI_DEVICENAME,
                path.as_mut_ptr().cast(),
                &mut chars,
            )
        } == ERROR_RESULT
        {
            continue;
        }
        path[chars as usize] = 0;

        let mut handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            // Some HID drivers deny read access but allow metadata queries.
            handle = unsafe {
                CreateFileW(
                    path.as_ptr(),
                    0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
        }
        let (manufacturer, product) = if handle == INVALID_HANDLE_VALUE {
            (String::new(), String::new())
        } else {
            let strings = unsafe { (hid_string(handle, false), hid_string(handle, true)) };
            unsafe { CloseHandle(handle) };
            strings
        };

        output.push(RawDevice {
            manufacturer,
            product,
            vendor: hid.dwVendorId as u16,
            product_id: hid.dwProductId as u16,
            version: hid.dwVersionNumber as u16,
            usage: hid.usUsage,
        });
    }
    Ok(output)
}

pub fn list_devices(targets: Vec<String>) -> Result<Vec<DeviceEntry>, String> {
    let saved = saved_guids(&targets);
    let raw_devices = unsafe { query_raw_devices()? };
    let mut seen = HashSet::new();
    let mut result = Vec::new();

    for raw in raw_devices {
        let derived = sdl_guid(raw.vendor, raw.product_id, &raw.product);
        if !seen.insert((
            raw.vendor,
            raw.product_id,
            raw.version,
            raw.usage,
            derived.clone(),
        )) {
            continue;
        }
        let existing = saved
            .get(&(raw.vendor, raw.product_id))
            .and_then(|values| values.first())
            .cloned();
        let (guid, kind, note) = match existing {
            Some(value) if value == derived => (
                value,
                "confirmed".to_string(),
                "Matches the GUID already used in your game files".to_string(),
            ),
            Some(value) => (
                value,
                "on-disk".to_string(),
                format!(
                    "Using the GUID already in your game files; Windows-derived value: {derived}"
                ),
            ),
            None => (
                derived.clone(),
                "derived".to_string(),
                "Generated in FFB Blaster's DirectInput GUID format; verify it in one game first"
                    .to_string(),
            ),
        };
        result.push(DeviceEntry {
            name: friendly_name(&raw),
            product_string: raw.product,
            vendor_id: raw.vendor,
            product_id: raw.product_id,
            version: raw.version,
            usage: raw.usage,
            derived_guid: derived,
            guid,
            note,
            kind,
        });
    }
    result.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_sdl_reference_algorithm() {
        // Standard CRC-16/IBM check value with an initial CRC of zero.
        assert_eq!(crc16(0, b"123456789"), 0xbb3d);
    }

    #[test]
    fn guid_layout_and_parser_agree() {
        let guid = sdl_guid(0x346e, 0x0004, "Wheel Base");
        let (_, vendor, product) = parse_guid(&guid).unwrap();
        assert_eq!(vendor, 0x346e);
        assert_eq!(product, 0x0004);
        assert!(guid.starts_with("0300"));
        assert_eq!(&guid[24..28], "0000");
    }

    #[test]
    fn moza_r5_matches_ffb_blaster_directinput_guid() {
        assert_eq!(
            sdl_guid(0x346e, 0x0004, "MOZA R5 Base"),
            "0300c3096e3400000400000000000000"
        );
    }

    #[test]
    fn rejects_malformed_guids() {
        assert!(parse_guid("not-a-guid").is_none());
        assert!(parse_guid("030000006e340000040000000000000z").is_none());
    }
}
