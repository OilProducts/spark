#!/bin/sh
# Seed codex auth from the read-only staging mount into a writable
# container-local home, mirroring spark's packaged-docker auth seeding.
codex_home="${HOME:-/home/chris}/.codex"
mkdir -p "$codex_home"
if [ -d /mnt/codex-auth ]; then
    for file in auth.json config.toml; do
        if [ -f "/mnt/codex-auth/$file" ] && [ ! -f "$codex_home/$file" ]; then
            cp "/mnt/codex-auth/$file" "$codex_home/$file"
            chmod 600 "$codex_home/$file"
        fi
    done
fi
exec "$@"
