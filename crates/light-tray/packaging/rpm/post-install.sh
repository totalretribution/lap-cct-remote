# 0.1.3 and older limited the radio helper to this group; any user can run it now
if getent group light-remote >/dev/null; then
    groupdel light-remote || true
fi
exit 0
