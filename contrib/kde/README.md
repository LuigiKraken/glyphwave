# KDE Plasma

`glyphwave setup` does all of this, lists the changes first and keeps the
originals for `glyphwave setup --remove`. By hand:

PowerDevil runs a command after an idle timeout, per power profile (`AC`,
`Battery`, `LowBattery`). In `~/.config/powerdevilrc`:

```ini
[AC][RunScript]
IdleTimeoutCommand=/home/you/.local/bin/glyphwave launch
RunScriptIdleTimeoutSec=300
```

and the same under `[Battery][RunScript]` for battery too. PowerDevil splits
the command like a shell would (quotes work, no pipes). Reload it without
logging out:

```sh
qdbus6 org.kde.Solid.PowerManagement /org/kde/Solid/PowerManagement reparseConfiguration
qdbus6 org.kde.Solid.PowerManagement /org/kde/Solid/PowerManagement refreshStatus
```

The later step is KDE's own timer, counted from the start of idle, so it
sits at the screensaver's start plus the extra minutes:

| then | where | keys |
|---|---|---|
| screen-off | `powerdevilrc`, `[AC][Display]` | `TurnOffDisplayWhenIdle=true`, `TurnOffDisplayIdleTimeoutSec=900` |
| sleep | `powerdevilrc`, `[AC][SuspendAndShutdown]` | `AutoSuspendAction=1`, `AutoSuspendIdleTimeoutSec=900` |
| lock | `kscreenlockerrc`, `[Daemon]` (both power states) | `Autolock=true`, `Timeout=15` (minutes) |

Dimming is set in each profile glyphwave runs in, because a dim at KDE's own
time would dim it partway through or before it starts. `dim = no` sets
`[AC][Display] DimDisplayWhenIdle=false`. `dim = yes` sets
`DimDisplayWhenIdle=true` with `DimDisplayIdleTimeoutSec` at the screensaver's
start, so it runs dimmed from the first frame. `dim = battery` (the default)
does that only under `[Battery]`, and sets `[AC]` like `dim = no`.

A key missing from the file means Plasma's default, and those fire early:
screen off after 10 min (5 on battery), sleep after 15 (10 on battery), lock
after 5. Anything at or before the screensaver's start hides or ends it;
setup warns about each one. After
editing the lock timer by hand, `qdbus6 org.freedesktop.ScreenSaver /ScreenSaver configure`.

glyphwave turns the terminal background black itself (OSC 11); a terminal
that ignores that keeps its own background. If Konsole's scrollbar or margin
shows, make a profile without them in Konsole and name it in
`~/.config/glyphwave/config`; `glyphwave launch` passes it on as `--profile`:

```ini
[terminal]
konsole_profile = MyProfile
```

glyphwave exits by itself when the KDE locker comes up (it watches
`org.freedesktop.ScreenSaver` on the session bus), so no lock hook is needed.

`lock_after` in the config: woken after that many minutes, glyphwave runs
`loginctl lock-session` and closes once KDE reports the lock screen active,
so you land on it. Nothing in KDE's own files changes for it. With
`then = lock`, KDE's lock timer still ends the screensaver at its time; for
a screensaver that runs on and locks only when you come back, use
`then = screen-off` or `none` and switch off Lock screen automatically in
System Settings > Screen Locking.

Sleep while music plays: players hold off sleep while they play. When they
let go, PowerDevil restarts the idle count, so the computer sleeps the full
timeout after the music stops, not straight away.

Starting it by hand: a video, or music in some players, holds off the idle
timer, so the screensaver never starts on its own. Setup writes
`~/.local/share/applications/glyphwave.desktop` (an app-menu entry you can
pin to the panel) running `glyphwave launch --now` (woken, it locks at once
when `lock_after` is set), with `X-KDE-Shortcuts=Meta+Ctrl+L` (`shortcut` in the
config). kglobalacceld reads that key when the menu database is rebuilt, so
setup runs `kbuildsycoca6` and the key works at once, with nothing written
to `kglobalshortcutsrc`. If that file already gives the key to something
else, setup binds none and says so; free it in System Settings > Keyboard >
Shortcuts and run setup again. A changed key takes effect at the next login.
