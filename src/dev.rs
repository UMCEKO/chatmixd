use std::{
    fs::{File, read_to_string},
    io::Write,
    thread,
};

const SS_VENDOR_ID: u16 = 0x1038;
const NOVA_PRO_PROD_ID: u16 = 0x12e0;

/// Parse `HID_ID=<bus>:<vid>:<pid>` from `/sys/class/hidraw/<device>/device/uevent`.
/// Returns `(vendor_id, product_id)` if found, else `None`.
fn read_hid_id(device: &str) -> Option<(u16, u16)> {
    let device_info = read_to_string(format!("/sys/class/hidraw/{device}/device/uevent")).ok()?;
    for line in device_info.lines() {
        let Some(hid_id) = line.strip_prefix("HID_ID=") else {
            continue;
        };
        let parts: Vec<&str> = hid_id.split(':').collect();
        let [_, id_vend, id_prod] = parts.as_slice() else {
            eprintln!("Malformed HID_ID field on {device}");
            return None;
        };
        let vid = u16::from_str_radix(id_vend, 16).ok()?;
        let pid = u16::from_str_radix(id_prod, 16).ok()?;
        return Some((vid, pid));
    }
    None
}

/// Whether the given hidraw device belongs to a SteelSeries product.
///
/// Used to skip hidraw nodes from other vendors (keyboards, mice, RGB
/// controllers...) whose HID reports can otherwise collide with the chatmix
/// opcode and produce phantom volume changes. See issue: Wooting analog
/// keyboards stream `[0x45, <hid_keycode>, <actuation>, <pad>]` reports which
/// match the chatmix pattern `[0x45, game, chat, 0]` whenever an analog key is
/// released, causing volumes to snap to nonsense values like (23, 0).
pub fn is_steelseries(device: &str) -> bool {
    matches!(read_hid_id(device), Some((SS_VENDOR_ID, _)))
}

pub fn determine_if_arctis_nova_pro(device: String) {
    let Some((vid, pid)) = read_hid_id(&device) else {
        eprintln!("Unable to read information on {device}.");
        return;
    };

    if vid != SS_VENDOR_ID {
        return;
    }

    if pid == NOVA_PRO_PROD_ID {
        println!("Arctis Nova Pro found (device /dev/{device})");
        send_init_to_nova_pro(device);
    }
}

fn send_init_to_nova_pro(device: String) {
    // Absolutely zero clue what exactly this does, especially since I don't own an Arctis Nova Pro
    // Someone with more expertise, please explain sometime later, if possible
    // -- birbwell
    let commands = [
        vec![0x06u8, 0x20],
        vec![0x06, 0x20],
        vec![0x06, 0x10],
        vec![0x06, 0x10],
        vec![0x06, 0x10],
        vec![0x06, 0x3b],
        vec![0x06, 0x8d, 0x01],
        vec![0x06, 0x20],
        vec![0x06, 0x20],
        vec![0x06, 0x20],
        vec![0x06, 0x80],
        vec![0x06, 0x3b],
        vec![0x06, 0x8d, 0x01],
        vec![0x06, 0x33, 0x14, 0x14, 0x14],
        vec![0x06, 0xc3, 0x00],
        vec![0x06, 0x2e, 0x00],
        vec![0x06, 0xc1, 0x00],
        vec![0x06, 0x85, 0x0a],
        vec![0x06, 0x37, 0xa0],
        vec![0x06, 0xb2],
        vec![0x06, 0x47, 0x64, 0x00, 0x64],
        vec![0x06, 0x83, 0x01],
        vec![0x06, 0x89, 0x00],
        vec![0x06, 0x27, 0x01],
        vec![0x06, 0xb3, 0x00],
        vec![0x06, 0x39, 0x00],
        vec![0x06, 0xbf, 0x05],
        vec![0x06, 0x43, 0x01],
        vec![0x06, 0x69, 0x00],
        vec![0x06, 0x3b, 0x00],
        vec![0x06, 0x8d, 0x01],
        vec![0x06, 0x49, 0x01],
        vec![0x06, 0xb7, 0x00],
        vec![0x06, 0xb7, 0x00],
        vec![0x06, 0xb7, 0x00],
        vec![0x06, 0x11],
        vec![0x06, 0x20, 0x00],
        vec![0x06, 0xb7, 0x00],
    ];

    let Ok(mut dev_f) = File::options().write(true).open(format!("/dev/{device}")) else {
        // Should never happen, since this is past the "Is Nova Pro" check, which should be writeable due to udev rules
        // If this is ever called, there's an issue with the udev rules
        eprintln!("Unable to open /dev/{device}");
        return;
    };

    // Do this asyncronously, to keep things moving if one of the devices times out (expected behaviour)
    thread::spawn(move || {
        for cmd in commands {
            if let Err(e) = dev_f.write(&cmd) {
                // Ignore timed out errors, but still return early.
                // This is because there are usually more than one steelseries devices.
                // As far as I can tell, one is read-only, the other is write-only.
                // I just write to both, since idk how to tell the difference, but the
                // read-only one will time out.
                if e.kind() == std::io::ErrorKind::TimedOut {
                    return;
                };

                eprintln!("{e:?}");
                return;
            };
        }
    });
}
