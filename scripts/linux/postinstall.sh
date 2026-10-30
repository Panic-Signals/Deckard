#!/bin/sh
# After the .deb or .rpm is installed: the udev rule applies to an MK3 already plugged
# in, and the desktop learns the entry, the icon and the .mxprj file type, without a
# log-out. Each step is skipped where its tool isn't there (a container, a minimal
# system) and never fails the install.
if command -v udevadm >/dev/null 2>&1; then
    udevadm control --reload-rules >/dev/null 2>&1 || true
    udevadm trigger --subsystem-match=usb --subsystem-match=hidraw >/dev/null 2>&1 || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications >/dev/null 2>&1 || true
fi
if command -v update-mime-database >/dev/null 2>&1; then
    update-mime-database /usr/share/mime >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor >/dev/null 2>&1 || true
fi
exit 0
