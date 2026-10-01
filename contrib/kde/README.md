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

A key missing from the file means Plasma's default, and those fire early:
dim after 5 min (2 on battery), screen off after 10 (5 on battery), sleep
after 15 (10 on battery), lock after 5. Anything at or before the
screensaver's start hides or ends it; setup warns about each one. After
editing the lock timer by hand, `qdbus6 org.freedesktop.ScreenSaver /ScreenSaver configure`.

Konsole needs a profile with a black background, no scrollbar and no
margin, or the screensaver shows a border. Setup writes one,
`~/.local/share/konsole/Glyphwave.profile` with its colour scheme, and names
it in `~/.config/glyphwave/config`:

```ini
[terminal]
konsole_profile = Glyphwave
```

glyphwave exits by itself when the KDE locker comes up (it watches
`org.freedesktop.ScreenSaver` on the session bus), so no lock hook is needed.
