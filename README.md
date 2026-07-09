# chatmixd

A SteelSeries ChatMix daemon for Linux. Reads the chatmix dial off the headset's
HID stream and splits audio into separate **Game** and **Chat** PipeWire/PulseAudio
sinks routed to your headset.

This is a hardened fork of [Birbwell/linuxmix](https://codeberg.org/Birbwell/linuxmix)
(MIT, 2025). The most user-visible change vs. upstream is that `chatmixd` filters
device discovery by USB vendor ID — it will not open hidraw nodes from non-SteelSeries
devices. Upstream opens every `/dev/hidraw*` and can be fooled by reports from other
gear (notably analog keyboards like the Wooting 60HE, whose per-key reports begin
with `0x45` — the same byte ChatMix uses as its opcode — and cause phantom volume
changes when an analog key is released). See [`docs/why-the-fork.md`](docs/why-the-fork.md)
for the full diagnosis.

## Requirements

- PipeWire with `pipewire-pulse` (PulseAudio compat layer)
- A supported SteelSeries headset with a ChatMix dial (Arctis Nova series, Arctis 7+,
  Nova Pro, etc.)
- `pactl` and `pw-cli` on `$PATH`

## Install

### Arch / pacman-based (AUR)

```sh
# with an AUR helper
yay -S chatmixd-git
# or paru, etc.

# or manually:
git clone https://aur.archlinux.org/chatmixd-git.git
cd chatmixd-git
makepkg -si
```

Then enable the user service:

```sh
systemctl --user enable --now chatmixd
```

### Fedora (COPR)

```sh
sudo dnf copr enable umceko/chatmixd
sudo dnf install chatmixd
systemctl --user enable --now chatmixd
```

### NixOS (flake)

Add the flake as an input and import the NixOS module, which installs the
binary, the udev rules, and the systemd user service:

```nix
{
  inputs.chatmixd.url = "github:UMCEKO/chatmixd";

  # in your NixOS configuration:
  imports = [ chatmixd.nixosModules.default ];
  services.chatmixd.enable = true;
}
```

On a channel-based (non-flake) system, the module also works imported
directly from a checkout — it builds the package from source on rebuild:

```nix
{
  imports = [ /path/to/chatmixd/nix/module.nix ];
  services.chatmixd.enable = true;
}
```

The service starts automatically on login; no `systemctl enable` needed.
To just try the daemon without installing: `nix run github:UMCEKO/chatmixd`
(note the udev rules granting hidraw access only apply via the module).

### From source

```sh
git clone https://github.com/UMCEKO/chatmixd.git
cd chatmixd
cargo build --release
sudo install -Dm755 target/release/chatmixd /usr/bin/chatmixd
sudo install -Dm644 dist/99-chatmixd.rules /etc/udev/rules.d/99-chatmixd.rules
sudo install -Dm644 dist/chatmixd.service /usr/lib/systemd/user/chatmixd.service
sudo udevadm control --reload-rules && sudo udevadm trigger
systemctl --user daemon-reload
systemctl --user enable --now chatmixd
```

After install, make sure your default sink is set to `Game` (e.g. in pavucontrol /
hyprpanel / kde-pulse-applet).

## Uninstall

```sh
systemctl --user disable --now chatmixd
sudo rm /usr/bin/chatmixd /etc/udev/rules.d/99-chatmixd.rules /usr/lib/systemd/user/chatmixd.service
sudo udevadm control --reload-rules
```

## How it works

`chatmixd` polls hidraw character devices for SteelSeries vendor ID `0x1038`
only. When a chatmix HID event arrives it parses `(game_vol, chat_vol)` from
the report and runs `pactl set-sink-volume` against the two null sinks it
creates on first event (`Game` and `Chat`). Both sinks are loopback-routed to
your physical Arctis sink, so adjusting the dial changes the relative mix of
the game vs chat sinks while keeping a single output device.

A blacklist file at `~/.config/chatmixd/blacklist.conf` (one node name per
line) lets you exclude specific PipeWire sinks from being considered as the
"physical" output. It's created on first run.

## Credit

Original concept and bulk of the implementation by **Birbwell** in
[linuxmix](https://codeberg.org/Birbwell/linuxmix). This fork carries the
vendor-ID filter, packaging changes, and a different name. The upstream MIT
license is preserved.
