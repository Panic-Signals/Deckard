#!/bin/sh
# After the .deb or .rpm is removed: the desktop forgets the entry, the icon and the
# file type. (On an upgrade the new version's postinstall runs after this.)
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
