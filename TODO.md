# glyphwave to-do

State at 1.0.0 (2026-10-06): the v0.9.9 VM findings are fixed and a full
regression pass passed on ubuntu-gnome, fedora-kde, mint-cinnamon,
arch-hyprland and debian-x11. Test kit and results:
~/Projects/vm-lab/kits/glyphwave/.

## Still to test

- GNOME and Fedora on real hardware (only tested in VMs so far).
- Real Slack, Discord, Teams and WhatsApp call notifications.
- Battery behaviour (on_battery, dim = battery) on a real laptop; the VMs
  have no battery.
- Media keys on Xfce (they did nothing in the VM, glyphwave or not).
- sway: setup and --remove text were only checked locally, never in a VM.
- Older Hyprland with hyprland.conf on a real install (only checked on 0.56
  with a .conf and by unit test).
- Ptyxis: a "Close Window? processes still running" dialog was left once
  after a wake and never came back in 6 tries. Watch for it on real GNOME.
- Whether hyprlock reads /etc/xdg/hypr/hyprlock.conf (setup assumes it does
  and stays quiet then).

## Could change

- xterm draws boxes for katakana, ♪ and ━: `-class glyphwave`
  (src/launch.rs:130) skips Debian's XTerm font setting. Likely fix: add
  `-fa monospace`. Keep the class, the sway window rule matches it.
- Idle hook for Cinnamon and Xfce without xidlehook (today: build it with
  cargo, ~400 MB of build deps):
  1. Cinnamon: reuse `idle-watch` on Muffin's IdleMonitor
     (org.cinnamon.Muffin.IdleMonitor), autostart entry like GNOME's, key via
     org.cinnamon.desktop.keybindings. ~30 lines, no polling. Recommended;
     check WatchFired actually fires first.
  2. Xfce: (a) MIT-SCREEN-SAVER idle query over the X socket, ~100-150
     lines, no new dependency; or (b) poll `xprintidle` every 10-15 s, ~30
     lines plus a package. Key via xfconf-query, ~30-40 lines with undo.
- light-locker (Xfce) answers GetActive false while locked, so
  `glyphwave launch` can start hidden behind its lock screen.
- Hyprland: when then_after <= start_after the lock moves to start_after + 1
  min; setup notes it but its one-line summary still shows the asked time.
- Setup only swaps the X11 locker while the config still has the default
  i3lock; a Cinnamon user with i3lock installed keeps i3lock.
- "Use these? [Y/n]" only treats an exact `n` as no (src/setup.rs:1025).
- Setup's diff for `then` lists the + line before the - line.
- Clippy: 12-14 warnings from newer lints (is_multiple_of,
  needless_range_loop) in older code.
- Distro packaging (PPA/apt repo, AUR -bin, COPR, nixpkgs), parked until
  after 1.0.

Known, can wait:
- Ptyxis's thin purple padding frame and gnome-terminal's scrollbar (no CLI
  options for either).
- Windows that open over glyphwave (Cinnamon: Mint's Update Manager welcome).
- Pointer visible on Hyprland/kitty.
- Progress bar stuck at the end of a looped track (4:00/4:00), likely
  Firefox's MPRIS position.

## Test kit (~/Projects/vm-lab/kits/glyphwave)

- `gw`'s envload should drop the ssh `LC_*` vars (or generate en_IE in the
  guests): with them xterm runs 8-bit and closes glyphwave by answering a
  glyph byte as a terminal query.
- PROTOCOL.md: `vm key equal` sends `=`, which wakes it by design; use
  `shift-equal` or `kp_add` for +.
- Guests' ~/kit copies were stale on arch, fedora and ubuntu; recopy before
  a run.
