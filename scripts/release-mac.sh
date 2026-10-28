#!/bin/zsh
# Builds, signs and notarizes the universal macOS release into release/.
# Needs, once per Mac (see docs/release.md):
#   - a "Developer ID Application" certificate in the login keychain;
#   - an app-specific Apple ID password stored as the generic keychain item "snapclone-notary"
#     (account = the Apple ID).
# Nothing secret is written to the repository or printed.
set -euo pipefail
cd "$(dirname "$0")/.."

fail() { print -u2 "release: $1"; exit 1; }

# Source tree: clean, on main, versions in step.
[[ -z "$(git status --porcelain)" ]] || fail "el árbol de trabajo tiene cambios sin commitear"
[[ "$(git branch --show-current)" == main ]] || fail "las releases salen de main"
version=$(node -p 'require("./package.json").version')
cargo_version=$(sed -nE 's/^version = "([^"]+)"/\1/p' src-tauri/Cargo.toml | head -1)
[[ "$version" == "$cargo_version" ]] || fail "package.json ($version) y Cargo.toml ($cargo_version) no coinciden"

# Credentials, read from the keychain.
identity=$(security find-identity -v -p codesigning | sed -nE 's/.*"(Developer ID Application: [^"]+)".*/\1/p' | head -1)
[[ -n "$identity" ]] || fail "no hay ningún certificado Developer ID Application en el llavero"
team_id=$(sed -nE 's/.*\(([A-Z0-9]{10})\)$/\1/p' <<<"$identity")
apple_id=$(security find-generic-password -s snapclone-notary 2>/dev/null | sed -nE 's/^ *"acct"<blob>="(.*)"$/\1/p')
[[ -n "$apple_id" ]] || fail "falta la contraseña de aplicación «snapclone-notary» en el llavero"
apple_password=$(security find-generic-password -s snapclone-notary -w)

# Checks.
npm run build
(cd src-tauri && cargo clippy --all-targets -q -- -D warnings && cargo test -q)

# Tauri signs the app, notarizes and staples it, then wraps it in the DMG.
export APPLE_SIGNING_IDENTITY="$identity" APPLE_ID="$apple_id" APPLE_PASSWORD="$apple_password" APPLE_TEAM_ID="$team_id"
npx tauri build --target universal-apple-darwin --bundles app,dmg

bundle_dir="src-tauri/target/universal-apple-darwin/release/bundle"
[[ -n "${CARGO_TARGET_DIR:-}" ]] && bundle_dir="$CARGO_TARGET_DIR/universal-apple-darwin/release/bundle"
app="$bundle_dir/macos/Snapclone.app"
dmg=$(ls "$bundle_dir"/dmg/Snapclone_${version}_*.dmg | head -1)

# The DMG itself is signed, notarized and stapled so it opens offline without warnings.
codesign --force --timestamp --sign "$identity" "$dmg"
xcrun notarytool submit "$dmg" --apple-id "$apple_id" --team-id "$team_id" --password "$apple_password" --wait
xcrun stapler staple "$dmg"
unset APPLE_PASSWORD apple_password

# Verification.
[[ "$(lipo -archs "$app/Contents/MacOS/snapclone")" == *x86_64*arm64* ]] || fail "el binario no es universal"
codesign --verify --deep --strict "$app"
xcrun stapler validate "$app"
xcrun stapler validate "$dmg"
spctl -a -vvv -t install "$dmg"

mkdir -p release
out="release/Snapclone_${version}_universal.dmg"
cp "$dmg" "$out"
(cd release && shasum -a 256 "${out:t}" > "${out:t}.sha256")
print "release: listo $out"
