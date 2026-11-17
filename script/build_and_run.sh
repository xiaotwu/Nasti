#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mode="${1:-run}"
case "$mode" in
  run|--build|--debug|--logs|--telemetry|--verify) ;;
  *) echo "Usage: $0 [--build|--debug|--logs|--telemetry|--verify]" >&2; exit 2 ;;
esac
if [ "$(uname -s)" != Darwin ]; then
  echo "The native NASTI preview requires macOS." >&2
  exit 1
fi

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root_dir/.nasti-build/target}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}"
scratch_dir="${NASTI_BUILD_SCRATCH:-$(dirname "$CARGO_TARGET_DIR")}"
bundle="$CARGO_TARGET_DIR/debug/bundle/macos/Nasti.app"

bundle_executable() {
  local name
  name="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$bundle/Contents/Info.plist")"
  printf '%s/Contents/MacOS/%s\n' "$bundle" "$name"
}

# Relaunch only the executable built in this run directory.
if [ "$mode" != "--build" ] && [ -d "$bundle" ]; then
  old_binary="$(bundle_executable)"
  running="$(ps -axo pid=,command= | awk -v path="$old_binary" '$2 == path && NF == 2 { print $1 }')"
  if [ -n "$running" ]; then
    while IFS= read -r pid; do
      kill -TERM "$pid" 2>/dev/null || true
    done <<< "$running"
  fi
fi

cd "$root_dir"
if [ ! -x node_modules/.bin/tauri ]; then
  pnpm install --frozen-lockfile
fi
cd "$root_dir/hosts/load-bearing-crew"
node "$root_dir/scripts/with-build-budget.mjs" "$scratch_dir" "$root_dir/packages/prism-glass-ui/dist" -- \
  "$root_dir/node_modules/.bin/tauri" build --debug --bundles app

test -d "$bundle"
if ! /usr/bin/codesign --verify --strict "$bundle" 2>/dev/null; then
  /usr/bin/codesign --force --sign - "$bundle"
fi
/usr/bin/codesign --verify --strict "$bundle"
echo "$bundle"
if [ "$mode" == "--build" ]; then exit 0; fi

binary="$(bundle_executable)"
if [ "$mode" == "--debug" ]; then exec lldb -- "$binary"; fi
if [ -n "${NASTI_DATA_DIR:-}" ]; then
  /usr/bin/open -n --env "NASTI_DATA_DIR=$NASTI_DATA_DIR" "$bundle"
else
  /usr/bin/open -n "$bundle"
fi
case "$mode" in
  --verify)
    for attempt in {1..10}; do
      if ps -axo command= | awk -v path="$binary" '$0 == path { found = 1 } END { exit !found }'; then
        exit 0
      fi
      sleep 1
    done
    echo "Nasti did not stay running." >&2
    exit 1
    ;;
  --logs|--telemetry)
    /usr/bin/log stream --info --style compact --predicate 'process == "load-bearing-crew" OR process == "Nasti"'
    ;;
esac
