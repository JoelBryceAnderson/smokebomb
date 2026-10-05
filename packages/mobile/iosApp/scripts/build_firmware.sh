#!/bin/bash
# Builds the firmware core (packages/firmware/ffi) as a static library for the
# iOS SDK Xcode is building for, and puts it where the app links it:
# build/firmware/<platform>/libsmokebomb_ffi.a. Xcode runs this before
# compiling Swift (see project.yml); you can also run it by hand.
#
# Needs Rust (https://rustup.rs). The iOS targets are added on first use.
set -euo pipefail

PLATFORM="${PLATFORM_NAME:-iphoneos}"
case "$PLATFORM" in
  iphonesimulator) TARGET=aarch64-apple-ios-sim ;;
  iphoneos) TARGET=aarch64-apple-ios ;;
  *) echo "error: build_firmware.sh: unsupported platform $PLATFORM" >&2; exit 1 ;;
esac

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../../../.." && pwd)"
OUT="$HERE/../build/firmware/$PLATFORM"

# Xcode's build phases don't read your shell profile.
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
if ! command -v cargo >/dev/null; then
  echo "error: Rust isn't installed. Install it from https://rustup.rs, then build again." >&2
  exit 1
fi
if command -v rustup >/dev/null && ! rustup target list --installed | grep -qx "$TARGET"; then
  rustup target add "$TARGET"
fi

# A clean environment: Xcode exports its iOS SDK settings (SDKROOT and the
# like), and the host-side build scripts would try to link against them.
env -i \
  HOME="$HOME" PATH="$PATH" USER="${USER:-}" \
  DEVELOPER_DIR="${DEVELOPER_DIR:-$(xcode-select -p)}" \
  IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-18.0}" \
  CARGO_TERM_COLOR=never \
  cargo build --manifest-path "$REPO/Cargo.toml" -p smokebomb-ffi --release --target "$TARGET"

mkdir -p "$OUT"
cp "$REPO/target/$TARGET/release/libsmokebomb_ffi.a" "$OUT/"
echo "firmware core: $OUT/libsmokebomb_ffi.a"
