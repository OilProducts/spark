#!/bin/sh
# Seed codex auth from the read-only staging mount into a writable
# container-local home, mirroring spark's packaged-docker auth seeding.
codex_home="${HOME:-/root}/.codex"
mkdir -p "$codex_home"
if [ -d /mnt/codex-auth ]; then
    for file in auth.json config.toml; do
        if [ -f "/mnt/codex-auth/$file" ] && [ ! -f "$codex_home/$file" ]; then
            cp "/mnt/codex-auth/$file" "$codex_home/$file"
            chmod 600 "$codex_home/$file"
        fi
    done
fi
# Flow commit nodes need a git identity; seed a neutral container-local
# one unless the session already provides it.
if ! git config --global user.email >/dev/null 2>&1; then
    git config --global user.name "MathLab Agent"
    git config --global user.email "mathlab-agent@localhost"
fi
exec "$@"
