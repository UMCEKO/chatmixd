# Why this fork exists

`linuxmix` (upstream) opens every `/dev/hidraw*` it can read and pattern-matches
the chatmix opcode (`0x45`) against every 4-byte window of every HID report.
That works when only a SteelSeries device is plugged in. It breaks when a
**different vendor** happens to emit a report that incidentally starts with
`0x45`.

## The Wooting case

The bug was first reproduced on a Wooting 60HE analog mechanical keyboard.
Wooting's analog HID interface emits per-key actuation reports framed as
`[0x45, <hid_keycode>, <actuation>, <padding>]` at high frequency. On key
release `actuation` and `padding` both go to `0`, producing a 4-byte window
of the form `[0x45, keycode, 0, 0]`.

That window matches upstream linuxmix's chatmix pattern
`[0x45, game_vol, chat_vol, 0]` exactly. linuxmix calls
`pactl set-sink-volume Game <keycode>` and `pactl set-sink-volume Chat 0`,
which slams the Game sink to a random percentage and Chat to zero — every time
that key is released.

For the `t` key (HID keycode `0x17` = 23 decimal), this manifests as the Game
sink suddenly jumping to **23%** on every release. Not on every keypress —
just on whichever key the analog frame happened to align on at the moment.
Intermittent, reproducible, and almost impossible to attribute without packet
captures.

`headsetcontrol --chatmix` returns the *real* dial position (typically `64`,
centered, meaning both channels at 100) all through the bug, which proves the
SteelSeries firmware is innocent. The bogus data is coming from a different
device entirely.

## The fix

The HID_ID sysfs attribute (`/sys/class/hidraw/<dev>/device/uevent`) exposes
`HID_ID=<bus>:<vid>:<pid>` for every hidraw node. The upstream code already
parses this for Arctis Nova Pro detection. `chatmixd` factors that parser into
a small `is_steelseries()` helper and gates `get_devices()` on it. Non-SteelSeries
hidraws are quietly skipped, so their report streams can never collide with
the chatmix opcode again.

## What that means in practice

- No phantom volume changes from analog keyboards, gaming mice with high-frequency
  reports, RGB controllers, or anything else with a non-SteelSeries USB VID.
- Real ChatMix events from your Arctis are unaffected.
- Real `HEADSET_POWER` events from your Arctis (which cleanup loopback sinks on
  power-off) still arrive correctly because the Arctis is still being read.

## Upstream status

This same fix was offered to `linuxmix` upstream as a pull request. Until /
unless it lands there, `chatmixd` exists as a hardened drop-in.
