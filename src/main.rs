use std::{
    fs::{File, OpenOptions},
    io::Read,
    process::{Command, Stdio},
    sync::{Mutex, OnceLock},
    thread, time,
};

mod dev;

const CHATMIX_CODE: u8 = 69; // Opcode for chatmix signal
const HEADSET_POWER: u8 = 185; // Opcode for power
const GAME: &str = "Game";
const CHAT: &str = "Chat";

static HOME_DIR: OnceLock<String> = OnceLock::new();

fn main() -> ! {
    if let Some(home_dir) = std::env::home_dir().map(|f| f.to_str().unwrap().to_string()) {
        if !std::fs::exists(&format!("{home_dir}/.config/chatmixd/blacklist.conf"))
            .unwrap_or_default()
        {
            std::fs::create_dir_all(&format!("{home_dir}/.config/chatmixd/blacklist.conf"))
                .unwrap();
            std::fs::File::options()
                .write(true)
                .create_new(true)
                .open(&format!("{home_dir}/.config/chatmixd/blacklist.conf"))
                .unwrap();
        }
        HOME_DIR.set(home_dir).unwrap();
    }

    loop {
        let devices = get_devices();
        let mut threads = vec![];
        for device in devices {
            threads.push(thread::spawn(|| read_device(device)));
        }

        while threads.iter().any(|f| !f.is_finished()) {
            thread::sleep(time::Duration::from_millis(100));
        }

        eprintln!("All threads exited unexpectedly -- Restarting...");
        thread::sleep(time::Duration::from_secs(2));
    }
}

fn get_devices() -> Vec<File> {
    std::fs::read_dir("/dev/")
        .unwrap()
        .filter_map(|f| {
            let t = f.unwrap().file_name().into_string().unwrap();
            if !t.starts_with("hidraw") {
                return None;
            }
            // Only open SteelSeries devices. Reading every hidraw on the system
            // means non-SteelSeries reports (e.g. analog-keyboard frames from a
            // Wooting that happen to start with `0x45`) get pattern-matched as
            // chatmix events, producing phantom volume changes.
            if !dev::is_steelseries(&t) {
                return None;
            }
            let dev = OpenOptions::new().read(true).open(format!("/dev/{t}")).ok()?;
            println!("Listening to /dev/{t}");
            dev::determine_if_arctis_nova_pro(t);
            Some(dev)
        })
        .collect()
}

fn get_device_id(device_name: &str) -> u32 {
    let mut pactl_out = Command::new("pactl")
        .arg("list")
        .arg("sinks")
        .arg("short")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    pactl_out.wait().unwrap();

    let grep_out = Command::new("grep")
        .arg(device_name)
        .stdin(Stdio::from(pactl_out.stdout.take().unwrap()))
        .output()
        .unwrap()
        .stdout;

    String::from_utf8(grep_out)
        .unwrap()
        .split_ascii_whitespace()
        .nth(0)
        .and_then(|sink_code| sink_code.parse::<u32>().ok())
        .unwrap()
}

fn in_blacklist(sink_name: &str) -> bool {
    let Some(home_dir) = HOME_DIR.get() else {
        return false;
    };
    let blacklist = std::fs::read_to_string(format!("{home_dir}/.config/chatmixd/blacklist.conf"))
        .unwrap_or_default();
    for line in blacklist.lines().map(|f| f.trim()) {
        if line.starts_with("#") {
            continue;
        }
        if line == sink_name {
            return true;
        }
    }

    false
}

fn get_default_sink() -> Option<u32> {
    let default_sink_name_bytes = Command::new("pactl")
        .arg("get-default-sink")
        .output()
        .unwrap()
        .stdout;

    let default_sink_name = String::from_utf8(default_sink_name_bytes)
        .map(|f| f.trim().to_owned())
        .unwrap();

    if default_sink_name.is_empty() {
        return None;
    }

    if in_blacklist(&default_sink_name) {
        return None;
    }

    Some(get_device_id(&default_sink_name))
}

fn configure_sinks() -> bool {
    // Prevent creating duplicate sinks, which would otherwise happen if the service is abruptly restarted
    cleanup_sinks();

    let Some(default_sink) = get_default_sink() else {
        return false;
    };

    let load_null_sink = |sink_name: &str| {
        Command::new("pactl")
            .arg("load-module")
            .arg("module-null-sink")
            .arg(format!("sink_name={sink_name}"))
            .stdout(Stdio::null())
            .spawn()
            .unwrap()
            .wait()
            .unwrap();
    };

    let load_loopback = |source: &str| {
        Command::new("pactl")
            .arg("load-module")
            .arg("module-loopback")
            .arg(format!("source={source}.monitor"))
            .arg(format!("sink={default_sink}"))
            .stdout(Stdio::null())
            .spawn()
            .unwrap()
            .wait()
            .unwrap();
    };

    load_null_sink(GAME);
    load_null_sink(CHAT);

    load_loopback(GAME);
    load_loopback(CHAT);

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
        Command::new("pactl")
            .arg("set-sink-volume")
            .arg(format!("{}", channel))
            .arg(format!("{}%", vol))
            .output()
            .unwrap();
    };

    let (code, game_vol, chat_vol) = match bytes {
        [code @ (CHATMIX_CODE | HEADSET_POWER), game_vol, chat_vol, 0] => {
            (code, game_vol, chat_vol)
        }
        [7, code @ (CHATMIX_CODE | HEADSET_POWER), game_vol, chat_vol] => {
            (code, game_vol, chat_vol)
        }
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
            return; // Return early bc we dont want to set the default sink to anything
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

    Command::new("pactl")
        .arg("set-default-sink")
        .arg("Game")
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
}

fn cleanup_sinks() {
    Command::new("pactl")
        .arg("unload-module")
        .arg("module-loopback")
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
        .wait()
        .unwrap();

    let destroy_sinks = |name: &str| {
        loop {
            let stat = Command::new("pw-cli")
                .arg("destroy")
                .arg(name)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
                .wait_with_output()
                .unwrap();

            // Prints to stderr whenever an error occurs, such as there being no sink by the given name
            // Despite printing to stderr, _the exit status is still 0_
            if stat.stderr.len() > 0 {
                break;
            }
        }
    };

    destroy_sinks(GAME);
    destroy_sinks(CHAT);
}
