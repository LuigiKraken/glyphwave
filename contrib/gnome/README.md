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
