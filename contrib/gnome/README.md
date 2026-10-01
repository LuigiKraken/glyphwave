# GNOME

GNOME has no setting that runs a command when idle, so `glyphwave setup`
adds a small watcher: `~/.config/autostart/glyphwave.desktop` runs
`glyphwave idle-watch` at login. It asks Mutter
(`org.gnome.Mutter.IdleMonitor`, `AddIdleWatch`) to signal after the idle
minutes and then runs `glyphwave launch`. No extension, no root. Mutter
doesn't count idle time while an app inhibits idle (a video playing), so
the screensaver doesn't start over a video.

The later step is GNOME's own setting, changed with `gsettings`, at the
screensaver's start plus the extra minutes. `--remove` puts the old values
back, or resets them if they were at their defaults.

| then | setting |
|---|---|
| screen-off | `org.gnome.desktop.session idle-delay` (seconds; GNOME also locks then if Screen Lock is on) |
| lock | the same, plus `org.gnome.desktop.screensaver lock-enabled true` and `lock-delay 0` |
| sleep | `org.gnome.settings-daemon.plugins.power sleep-inactive-ac-timeout` / `-battery-timeout` and `-type 'suspend'` |

GNOME's default Screen Blank is 5 minutes, which would end a screensaver
that starts at 5; setup warns when that's the case. Blank and lock are one
timer for both power states.

The terminal is the first installed of the launcher's list; on Fedora that
is usually Ptyxis (`ptyxis -s --fullscreen`), or gnome-terminal. glyphwave
exits when GNOME's lock screen comes up (`org.gnome.ScreenSaver`).

`lock_after` in the config: woken after that many minutes, glyphwave runs
`loginctl lock-session` and closes once GNOME's lock screen is up. With
`then = lock`, GNOME's blank-and-lock timer still ends the screensaver at
its time; `then = none` and Screen Blank set to Never leave it running until
you come back.

Sleep while music plays: players hold off sleep while they play. When they
let go, gnome-settings-daemon sets its sleep timer again, counted from your
last input, so if you've been away longer than the timeout it sleeps right
away. (An app that holds off idle instead, like a video, restarts the count.)

Starting it by hand: a video, or music in some players, holds off idle, so
the screensaver never starts on its own. Setup adds an app-menu entry
(`~/.local/share/applications/glyphwave.desktop`, pin it to the dock) and a
custom shortcut running `glyphwave launch --now`, `shortcut` in the config (Meta+Ctrl+L, `<Super><Control>l`):
its path goes into `org.gnome.settings-daemon.plugins.media-keys
custom-keybindings`, with `name`, `command` and `binding` under
`.../custom-keybindings/glyphwave/`. It shows in Settings > Keyboard >
Custom Shortcuts; `--remove` puts the list back as it was.

Fedora: Workstation (GNOME) and the KDE spin should need nothing extra, but
that is still to be tried on a real Fedora install. The binary in
`~/.local/bin` also suits Silverblue and the other Atomic variants, where
`/usr` is read-only.
