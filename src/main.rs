use std::{
    fs::{File, OpenOptions},
    io::Read,
    process::{Child, Command, Stdio},
    sync::{LazyLock, Mutex, MutexGuard},
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

/// Long-lived `pw-loopback` child processes — one per chatmix channel. Each
/// presents its capture side as a routable virtual sink (`Game`/`Chat`) and
/// feeds the headset directly, so the whole signal path lives in the headset's
/// single driver domain. This replaces the old `module-null-sink` +
/// `module-loopback` pair, which put each channel on PipeWire's dummy-timer
/// driver and then bridged it back to the device asynchronously — an extra
/// scheduling hop plus a resampler buffer. Killing a child tears its sink down.
static LOOPBACKS: Mutex<Vec<Child>> = Mutex::new(Vec::new());

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
                .inspect_err(|e| {
                    eprintln!("Cannot open /dev/{name} (check udev rules / permissions): {e}")
                })
                .ok()?;
            println!("Listening to /dev/{name}");
            dev::determine_if_arctis_nova_pro(name);
            Some(dev)
        })
        .collect()
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
/// matches SteelSeries, returning its node name. The chatmix routing target is
/// the user's headset, not whatever PipeWire currently considers the system
/// default — those can diverge (e.g. HDMI audio becomes default after a monitor
/// wake, or the headset hasn't been promoted to default yet). Iterating sinks
/// by vendor ID makes the target deterministic. The node name (not the numeric
/// index) is what `pw-loopback -P` needs to attach to.
fn find_steelseries_sink() -> Option<String> {
    let out = Command::new("pactl")
        .arg("list")
        .arg("sinks")
        .output()
        .inspect_err(|e| eprintln!("Failed to run `pactl list sinks`: {e}"))
        .ok()?;
    let stdout = String::from_utf8(out.stdout).ok()?;

    let mut current_name: Option<String> = None;
    let mut current_is_ss = false;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Sink #") {
            if current_is_ss {
                return current_name;
            }
            current_name = None;
            current_is_ss = false;
        } else if let Some(name) = trimmed.strip_prefix("Name: ") {
            current_name = Some(name.to_owned());
        } else if trimmed.starts_with("device.vendor.id") && trimmed.contains(SS_VENDOR_ID_PROP) {
            // pactl renders properties as: `device.vendor.id = "0x1038"`
            current_is_ss = true;
        }
    }
    if current_is_ss { current_name } else { None }
}

fn get_default_sink() -> Option<String> {
    // Prefer a SteelSeries-owned sink for the chatmix routing target. Falls
    // back to the system default sink only if no SteelSeries sink is present
    // (e.g. headset entirely unplugged), in which case the original
    // default-sink + blacklist behaviour applies.
    if let Some(name) = find_steelseries_sink() {
        return Some(name);
    }

    let out = Command::new("pactl")
        .arg("get-default-sink")
        .output()
        .inspect_err(|e| eprintln!("Failed to run `pactl get-default-sink`: {e}"))
        .ok()?;
    let default_sink_name = String::from_utf8(out.stdout).ok()?.trim().to_owned();

    if default_sink_name.is_empty() || in_blacklist(&default_sink_name) {
        return None;
    }

    Some(default_sink_name)
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

fn lock_loopbacks() -> MutexGuard<'static, Vec<Child>> {
    // The lock only ever guards Vec<Child> mutation; a panic mid-mutation can't
    // leave the Vec in a torn state, so recovering from poison is safe.
    LOOPBACKS.lock().unwrap_or_else(|p| p.into_inner())
}

/// Spawn one `pw-loopback` whose capture side is the routable virtual sink
/// `name` (Game/Chat) and whose playback side feeds `target` (the headset).
/// Latency is left unpinned so the node follows the graph quantum: latency-
/// sensitive apps (e.g. osu!) pull it down, idle playback relaxes it — no fixed
/// floor forced on every other app sharing the headset.
fn spawn_loopback(name: &str, target: &str) -> Option<Child> {
    Command::new("pw-loopback")
        .arg("-P")
        .arg(target)
        .arg("--capture-props")
        .arg(format!(
            "media.class=Audio/Sink node.name={name} node.description={name} audio.position=[FL,FR]"
        ))
        .arg("--playback-props")
        .arg("stream.dont-remix=true")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .inspect_err(|e| eprintln!("Failed to spawn pw-loopback for {name}: {e}"))
        .ok()
}

fn sink_exists(name: &str) -> bool {
    Command::new("pactl")
        .args(["list", "sinks", "short"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.lines().any(|l| l.split_whitespace().nth(1) == Some(name)))
        .unwrap_or(false)
}

/// `pw-loopback` registers its sink asynchronously, so poll briefly (up to ~1s)
/// for it to appear before we depend on it.
fn wait_for_sink(name: &str) -> bool {
    for _ in 0..50 {
        if sink_exists(name) {
            return true;
        }
        thread::sleep(time::Duration::from_millis(20));
    }
    false
}

fn configure_sinks() -> bool {
    // Prevent creating duplicate sinks, which would otherwise happen if the service is abruptly restarted
    cleanup_sinks();

    let Some(target) = get_default_sink() else {
        return false;
    };

    {
        let mut guard = lock_loopbacks();
        for name in [GAME, CHAT] {
            if let Some(child) = spawn_loopback(name, &target) {
                guard.push(child);
            }
        }
    }

    // Both sinks must come up; if either failed to spawn or never registered,
    // tear the partial state back down so we retry cleanly on the next packet.
    if !wait_for_sink(GAME) || !wait_for_sink(CHAT) {
        eprintln!("chatmix sinks did not come up; tearing down partial state");
        cleanup_sinks();
        return false;
    }

    // Promote Game to default sink ONCE on (re)configuration so generic
    // applications route through the chatmix split. Previously this was
    // re-issued on every HID packet, which clobbered the user's chosen
    // default sink on every micro-twist of the chatmix dial.
    pactl_run(&["set-default-sink", GAME]);

    true
}

fn read_device(mut file: File) {
    let mut buf = [0u8; 4];
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
    // Kill the pw-loopback children we own. Each owns exactly one Game/Chat
    // loopback-sink, which disappears when its process exits. This is scoped to
    // our own processes — unlike the previous `pactl unload-module
    // module-loopback`, which (with no module index) unloaded EVERY loopback on
    // the system, clobbering unrelated ones the user may run.
    {
        let mut guard = lock_loopbacks();
        for mut child in guard.drain(..) {
            let _ = child.kill();
            let _ = child.wait(); // reap so we don't leave zombies
        }
    }

    // Safety net: a previous daemon instance that was hard-killed (so its
    // children outlived it, or its Drop never ran) may have left orphaned
    // Game/Chat sinks. Destroy any lingering ones by name.
    destroy_orphan_sinks(GAME);
    destroy_orphan_sinks(CHAT);
}

fn destroy_orphan_sinks(name: &str) {
    // pw-cli destroy prints to stderr (but still exits 0) when no node by that
    // name remains. Loop until that happens — bounded, so a persistent error
    // can't hang the daemon.
    for _ in 0..16 {
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
        if !out.stderr.is_empty() {
            break;
        }
    }
}
