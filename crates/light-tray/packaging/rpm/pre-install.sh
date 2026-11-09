# The group that may run the radio helper; it must exist before the helper is unpacked.
getent group light-remote >/dev/null || groupadd -r light-remote
exit 0
