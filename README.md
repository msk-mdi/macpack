# macpack

## Install

With Homebrew:

```sh
brew install msk-mdi/tap/macpack
```

Or download the universal binary (Apple silicon and Intel) from the [latest release](https://github.com/msk-mdi/macpack/releases/latest):

```sh
curl -L https://github.com/msk-mdi/macpack/releases/latest/download/macpack-v1.0.0-universal-apple-darwin.tar.gz | tar -xz
sudo mv macpack /usr/local/bin/
```

The binary isn't notarized; if you download it with a browser, clear the quarantine flag with `xattr -d com.apple.quarantine macpack`.

Or build from source:

```sh
cargo install --git https://github.com/msk-mdi/macpack
```

## Use

```sh
macpack                      # interactive: export, apply, or restore
macpack export               # open the picker straight away
macpack apply profile.toml   # review the diff, deselect anything, apply
macpack restore              # review and undo the most recent apply
```

Non-interactive, for scripts:

```sh
macpack list [--category Dock]
macpack export --all -o ~/my-mac.toml
macpack export --ids dock,finder.pathbar,keyboard.key-repeat -o ~/my-mac.toml
macpack apply ~/my-mac.toml --dry-run
macpack apply ~/my-mac.toml --yes [--no-restart]
macpack backups
macpack restore 20261014-221000.123 --yes
```

`--ids` takes setting ids or whole category names (see `macpack list`).

### Picker keys

| Key | Action |
| --- | --- |
| ↑ ↓ / j k | move |
| ← → / Tab | switch between categories and settings |
| Space | toggle setting (on a category: toggle the whole category) |
| a / n | select all in view / clear selection |
| / | filter across all categories |
| Enter | continue |
| Esc / q | back |

## Profiles

A profile is TOML you can read, edit and commit to a dotfiles repo:

```toml
version = 1
created = "2026-10-14T20:24:42Z"
source_os = "27.0.1"

[[settings]]
id = "dock.autohide"
value = true

[[settings]]
id = "dock.orientation"   # no value: key is unset, so the system default applies
```

Settings are referenced by macpack id, not raw domain/key, so a profile can only ever touch settings in macpack's catalog ([src/catalog.rs](src/catalog.rs)). Unknown ids and values of the wrong type are reported and skipped.

## Applying

1. Each setting in the profile is compared with this Mac; matching ones are left alone.
2. You review the changes and untick any you don't want.
3. The current values of the selected settings are saved to `~/Library/Application Support/macpack/backups/` — a backup is itself a profile, so restoring is just applying it.
4. Values are written with `defaults write` (or `defaults delete` for unset), then Dock, Finder or SystemUIServer are restarted as needed. Keyboard, trackpad and appearance settings may need a logout or app relaunch.

## Adding settings

Add an entry to `CATALOG` in [src/catalog.rs](src/catalog.rs): a namespaced id, category, label, preference domain and key, value kind, the process to restart (if any) and a one-line description. Use `.host()` for `-currentHost` preferences.

## Development

```sh
cargo test     # includes a round-trip against the real `defaults` in the com.macpack.selftest domain
cargo clippy --all-targets
```

### Releasing

Bump `version` in `Cargo.toml`, commit, then push a matching tag:

```sh
git tag v1.0.1 && git push origin v1.0.1
```

The [release workflow](.github/workflows/release.yml) runs the tests, builds Apple silicon, Intel and universal binaries, and publishes them with checksums to GitHub Releases, then updates the formula in [msk-mdi/homebrew-tap](https://github.com/msk-mdi/homebrew-tap) (needs a `HOMEBREW_TAP_TOKEN` secret with write access to that repo).
