# KDE Plasma

PowerDevil runs a command after an idle timeout. Add to `~/.config/powerdevilrc`:

```ini
[AC][RunScript]
IdleTimeoutCommand=/path/to/glyphwave-idle
RunScriptIdleTimeoutSec=300
```

and add the same under `[Battery][RunScript]` if you want it on battery too.
PowerDevil reads the file at login.

Konsole needs a profile with a black background, no scrollbar and no padding,
or the screensaver shows a border. Set `GLYPHWAVE_TERM=konsole` and
`GLYPHWAVE_KONSOLE_PROFILE=<name>` in `~/.config/plasma-workspace/env/glyphwave.sh`
so PowerDevil's environment has them. Without them, the first terminal the
launcher finds is used.

glyphwave exits by itself when the KDE locker comes up (it watches
`org.freedesktop.ScreenSaver` on the session bus), so no lock hook is needed.
