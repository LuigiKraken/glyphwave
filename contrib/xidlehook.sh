#!/bin/sh
# X11 window managers (i3, bspwm, openbox, …): start from your session
# autostart. `glyphwave setup` prints this with your times filled in.
# xidlehook's timers count from the one before: lock 10 min after the start.
# No --not-when-fullscreen: the screensaver is itself a fullscreen window, so
# the lock timer would never fire. --not-when-audio would stop it ever
# showing the music themes, so it's left off too.
exec xidlehook \
    --timer 300 'glyphwave launch' '' \
    --timer 600 'glyphwave launch --stop; i3lock -c 000000' ''
