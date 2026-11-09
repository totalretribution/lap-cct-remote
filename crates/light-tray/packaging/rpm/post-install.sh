# Adds the person installing (through sudo or pkexec) to the light-remote group, so they don't have to.
user=""
if [ -n "${SUDO_USER:-}" ]; then
    user="$SUDO_USER"
elif [ -n "${PKEXEC_UID:-}" ]; then
    user="$(getent passwd "$PKEXEC_UID" | cut -d: -f1)"
fi
if [ -n "$user" ] && [ "$user" != root ] && ! id -nG "$user" | tr ' ' '\n' | grep -qx light-remote; then
    usermod -aG light-remote "$user"
    echo "light-remote: added $user to the light-remote group; log out and back in to use the light."
fi
echo "light-remote: other users need adding with: sudo usermod -aG light-remote <user>"
exit 0
