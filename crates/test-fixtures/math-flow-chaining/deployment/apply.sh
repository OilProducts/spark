#!/bin/sh
set -eu

if [ "$#" -lt 3 ] || [ "$2" != "--" ]; then
  echo "usage: apply.sh CONFIG_DIR -- INTEGRATION_TEST [ARGS...]" >&2
  exit 2
fi

config_dir=$1
shift 2
trigger_dir=$config_dir/triggers
definitions_dir=$(CDPATH= cd -- "$(dirname -- "$0")/triggers" && pwd)
mkdir -p "$trigger_dir"

install_definition() {
  source=$1
  destination=$2
  enabled=$3
  temporary=$destination.tmp.$$
  sed "s/^enabled = true$/enabled = $enabled/" "$source" >"$temporary"
  mv "$temporary" "$destination"
}

# Keep both consumers inert while the obsolete timers are removed and tested.
for id in tuza-chain bsd-chain; do
  install_definition "$definitions_dir/$id.toml" "$trigger_dir/$id.toml" false
done
for id in tuza-ignite bsd-ignite; do
  rm -f "$trigger_dir/$id.toml"
done

"$@"

# The gate passed: publish exactly one completed-only chain for each project.
for id in tuza-chain bsd-chain; do
  install_definition "$definitions_dir/$id.toml" "$trigger_dir/$id.toml" true
done

for id in tuza-ignite bsd-ignite; do
  test ! -e "$trigger_dir/$id.toml"
done
for id in tuza-chain bsd-chain; do
  definition=$trigger_dir/$id.toml
  test "$(grep -c '^enabled = true$' "$definition")" -eq 1
  test "$(grep -Fxc 'statuses = ["completed"]' "$definition")" -eq 1
done
