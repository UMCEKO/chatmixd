use std::{
    fs::{File, OpenOptions},
    io::Read,
    process::{Command, Stdio},
    sync::{LazyLock, Mutex},
    thread, time,
};

mod dev;

const CHATMIX_CODE: u8 = 69; // Opcode for chatmix signal
const HEADSET_POWER: u8 = 185; // Opcode for power
const GAME: &str = "Game";
const CHAT: &str = "Chat";
const SS_VENDOR_ID_PROP: &str = "0x1038"; // SteelSeries USB VID, as printed by `pactl list sinks`

/// `$HOME` resolved at first access. `None` if unset or non-UTF-8 — in which
/// case blacklist support is silently disabled (and `in_blacklist` returns
/// false for every sink name).
static HOME_DIR: LazyLock<Option<String>> = LazyLock::new(|| {
    std::env::home_dir().and_then(|p| p.into_os_string().into_string().ok())
});

fn main() -> ! {
    if let Some(home_dir) = HOME_DIR.as_deref() {
        let config_dir = format!("{home_dir}/.config/chatmixd");
        let blacklist_path = format!("{config_dir}/blacklist.conf");
        if !std::fs::exists(&blacklist_path).unwrap_or_default() {
            // Create the parent directory (NOT the file path) and then touch the
            // blacklist file. Upstream's version called `create_dir_all` with the
            // blacklist path, which created a directory at that path and made the
            // subsequent open() fail.
            if let Err(e) = std::fs::create_dir_all(&config_dir) {
                eprintln!("Unable to create config dir {config_dir}: {e}");
            } else if let Err(e) = File::options()
                .write(true)
                .create_new(true)
                .open(&blacklist_path)
            {
                eprintln!("Unable to create blacklist file {blacklist_path}: {e}");
            }
        }
    } else {
        eprintln!("No HOME directory found; blacklist support disabled.");
    }

    loop {
        let devices = get_devices();
        // `thread::scope` auto-joins all spawned threads at the end of the
        // block, so we don't need a busy-poll + sleep to wait for any thread
        // to finish, and we don't need 'static-bound closures.
        thread::scope(|s| {
            for device in devices {
                s.spawn(|| read_device(device));
            }
        });

        eprintln!("All threads exited unexpectedly -- Restarting...");
        thread::sleep(time::Duration::from_secs(2));
    }
}

fn get_devices() -> Vec<File> {
    let entries = match std::fs::read_dir("/dev/") {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Cannot enumerate /dev: {e}");
            return Vec::new();
        }
    };

    entries
        .filter_map(|f| {
            let entry = f.ok()?;
            let name = entry.file_name().into_string().ok()?;
            if !name.starts_with("hidraw") {
                return None;
            }
            // Only open SteelSeries devices. Reading every hidraw on the system
            // means non-SteelSeries reports (e.g. analog-keyboard frames from a
            // Wooting that happen to start with `0x45`) get pattern-matched as
            // chatmix events, producing phantom volume changes.
            if !dev::is_steelseries(&name) {
                return None;
            }
            let dev = OpenOptions::new()
                .read(true)
                .open(format!("/dev/{name}"))
                .ok()?;
            println!("Listening to /dev/{name}");
            dev::determine_if_arctis_nova_pro(name);
            Some(dev)
        })
        .collect()
}

fn get_device_id(device_name: &str) -> Option<u32> {
    let pactl_out = Command::new("pactl")
        .arg("list")
        .arg("sinks")
        .arg("short")
        .output()
        .ok()?;
    let stdout = String::from_utf8(pactl_out.stdout).ok()?;

    stdout
        .lines()
        .find(|line| line.contains(device_name))
        .and_then(|line| line.split_ascii_whitespace().next())
        .and_then(|sink_code| sink_code.parse::<u32>().ok())
}

fn in_blacklist(sink_name: &str) -> bool {
    let Some(home_dir) = HOME_DIR.as_deref() else {
        return false;
    };
    let blacklist = std::fs::read_to_string(format!("{home_dir}/.config/chatmixd/blacklist.conf"))
        .unwrap_or_default();
    blacklist
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .any(|l| l == sink_name)
}

/// Scan `pactl list sinks` (long form) for a sink whose USB vendor ID
/// matches SteelSeries. The chatmix routing target is the user's headset,
/// not whatever PipeWire currently considers the system default — those
/// can diverge (e.g. HDMI audio becomes default after a monitor wake, or
/// the headset hasn't been promoted to default yet). Iterating sinks by
/// vendor ID makes the target deterministic.
fn find_steelseries_sink() -> Option<u32> {
    let out = Command::new("pactl")
        .arg("list")
        .arg("sinks")
        .output()
        .ok()?;
    let stdout = String::from_utf8(out.stdout).ok()?;

    let mut current_id: Option<u32> = None;
    let mut current_is_ss = false;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Sink #") {
            if current_is_ss {
                return current_id;
            }
            current_id = rest.parse::<u32>().ok();
            current_is_ss = false;
        }
        // pactl renders properties as: `device.vendor.id = "0x1038"`
        if trimmed.starts_with("device.vendor.id") && trimmed.contains(SS_VENDOR_ID_PROP) {
            current_is_ss = true;
        }
    }
    if current_is_ss { current_id } else { None }
}

fn get_default_sink() -> Option<u32> {
    // Prefer a SteelSeries-owned sink for the chatmix routing target. Falls
    // back to the system default sink only if no SteelSeries sink is present
    // (e.g. headset entirely unplugged), in which case the original
    // default-sink + blacklist behaviour applies.
    if let Some(id) = find_steelseries_sink() {
        return Some(id);
    }

    let out = Command::new("pactl").arg("get-default-sink").output().ok()?;
    let default_sink_name = String::from_utf8(out.stdout).ok()?.trim().to_owned();

    if default_sink_name.is_empty() || in_blacklist(&default_sink_name) {
        return None;
    }

    get_device_id(&default_sink_name)
}

/// Run `pactl <args>` for side-effect; log on failure, don't panic.
fn pactl_run(args: &[&str]) {
    match Command::new("pactl")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if !status.success() => {
            eprintln!("pactl {} exited with {status}", args.join(" "));
        }
        Err(e) => eprintln!("Failed to spawn pactl {}: {e}", args.join(" ")),
        _ => {}
    }
}

fn configure_sinks() -> bool {
    // Prevent creating duplicate sinks, which would otherwise happen if the service is abruptly restarted
    cleanup_sinks();

    let Some(default_sink) = get_default_sink() else {
        return false;
    };

    let sink_id = default_sink.to_string();

    pactl_run(&["load-module", "module-null-sink", &format!("sink_name={GAME}")]);
    pactl_run(&["load-module", "module-null-sink", &format!("sink_name={CHAT}")]);
    pactl_run(&[
        "load-module",
        "module-loopback",
        &format!("source={GAME}.monitor"),
        &format!("sink={sink_id}"),
    ]);
    pactl_run(&[
        "load-module",
        "module-loopback",
        &format!("source={CHAT}.monitor"),
        &format!("sink={sink_id}"),
    ]);

    // Promote Game to default sink ONCE on (re)configuration so generic
    // applications route through the chatmix split. Previously this was
    // re-issued on every HID packet, which clobbered the user's chosen
    // default sink on every micro-twist of the chatmix dial.
    pactl_run(&["set-default-sink", GAME]);

    true
}

fn read_device(mut file: File) {
    let mut buf = [8u8; 4];
    while let Ok(()) = file.read_exact(&mut buf) {
        process_bytes(buf);
    }
}

fn process_bytes(bytes: [u8; 4]) {
    static IS_CONF: Mutex<bool> = Mutex::new(false);

    let set_volume = |channel: &str, vol: u8| {
        pactl_run(&["set-sink-volume", channel, &format!("{vol}%")]);
    };

    let (code, game_vol, chat_vol) = match bytes {
        [code @ (CHATMIX_CODE | HEADSET_POWER), game_vol, chat_vol, 0] => (code, game_vol, chat_vol),
        [7, code @ (CHATMIX_CODE | HEADSET_POWER), game_vol, chat_vol] => (code, game_vol, chat_vol),
        _ => return,
    };

    match code {
        CHATMIX_CODE => {
            if let Ok(mut conf) = IS_CONF.lock() {
                if !*conf {
                    *conf = configure_sinks();
                }

                if *conf {
                    set_volume(GAME, game_vol);
                    set_volume(CHAT, chat_vol);
                }
            }
        }
        HEADSET_POWER if game_vol == 2 => {
            // Power off
            if let Ok(mut conf) = IS_CONF.lock()
                && *conf
            {
                cleanup_sinks();
                *conf = false;
            }
        }
        HEADSET_POWER if game_vol == 3 => {
            // Power on
            if let Ok(mut conf) = IS_CONF.lock()
                && !*conf
            {
                *conf = configure_sinks();
            }
        }
        _ => unreachable!(),
    }
    // `pactl set-default-sink Game` deliberately runs ONLY inside
    // configure_sinks() (where Game and Chat are newly created). Doing it
    // here on every HID packet would clobber whatever default sink the user
    // (or PipeWire on hotplug) has set, on every twist of the chatmix dial.
}

fn cleanup_sinks() {
    pactl_run(&["unload-module", "module-loopback"]);

    let destroy_sinks = |name: &str| {
        loop {
            let out = match Command::new("pw-cli")
                .arg("destroy")
                .arg(name)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
            {
                Ok(o) => o,
                Err(e) => {
                    eprintln!("Failed to spawn pw-cli destroy {name}: {e}");
                    return;
                }
            };

            // pw-cli prints to stderr whenever an error occurs (e.g. no sink by
            // that name). Despite that, the exit status is still 0. Loop until
            // stderr is non-empty, meaning no more instances of the sink remain.
            if !out.stderr.is_empty() {
                break;
            }
        }
    };

    destroy_sinks(GAME);
    destroy_sinks(CHAT);
}
