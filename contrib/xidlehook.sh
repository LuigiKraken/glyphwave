#!/bin/sh
# X11 window managers (i3, bspwm, openbox, …): start from your session
# autostart. --not-when-fullscreen skips it while a video plays fullscreen;
# --not-when-audio would stop it ever showing the music themes, so it's left off.
exec xidlehook --not-when-fullscreen \
    --timer 300 'glyphwave-idle' '' \
    --timer 600 'glyphwave-idle --stop; i3lock -c 000000' ''
