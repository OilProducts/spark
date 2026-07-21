#!/bin/sh
# Seed codex auth from the read-only staging mount into a writable
# container-local home, mirroring spark's packaged-docker auth seeding.
mkdir -p /home/chris/.codex
if [ -d /mnt/codex-auth ]; then
    for file in auth.json config.toml; do
        if [ -f "/mnt/codex-auth/$file" ] && [ ! -f "/home/chris/.codex/$file" ]; then
            cp "/mnt/codex-auth/$file" "/home/chris/.codex/$file"
            chmod 600 "/home/chris/.codex/$file"
        fi
    done
fi
exec "$@"
