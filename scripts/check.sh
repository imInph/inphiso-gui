#!/usr/bin/env bash
# Everything CI runs, in one go. Run before committing.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --all --check
(cd app && node scripts/sidecar.mjs --debug >/dev/null)
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --quiet
(cd app && npm run --silent typecheck)

# Cross-check the OS-specific code when the targets are installed.
for target in x86_64-pc-windows-msvc x86_64-unknown-linux-gnu; do
  if rustup target list --installed | grep -qx "$target"; then
    cargo clippy -p inphiso-platform -p inphiso-helper --target "$target" --all-targets -- -D warnings
  fi
done

echo "all checks passed"
