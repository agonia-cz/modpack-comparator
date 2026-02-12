# Modpack Comparator

Minecraft modpack comparator with a native GUI — scans JAR metadata, creates snapshots, and generates changelogs.

Built with Rust and [egui](https://github.com/emilk/egui).

## Features

- Scans `fabric.mod.json` / `quilt.mod.json` metadata from JAR files
- Creates JSON snapshots of your mods folder
- Compares snapshots and detects: new, updated, removed, disabled, and re-enabled mods
- Generates Markdown changelogs ready to paste into Discord or GitHub
- Auto-detects [Modrinth App](https://modrinth.com/app) profiles with customizable aliases
- Snapshot history — compare any two previous snapshots
- Async scanning — GUI stays responsive during scan
- Dark theme, native Windows GUI (no browser, no Electron)

## Screenshot

![Modpack Comparator](app-logo.png)

## Build

Requires [Rust](https://rustup.rs/) toolchain.

```bash
cargo build --release
```

The binary will be at `target/release/porovnavac.exe`.

## Usage

1. Launch `porovnavac.exe`
2. Select a Modrinth profile from the dropdown (or browse to a custom mods folder)
3. Set your pack name, edition, and version
4. Click **Skenovat a porovnat**
5. View results in the **Results** tab or copy the Markdown changelog

## Profile Aliases

On first run, the app creates `aliases.json` in `%APPDATA%/ModrinthApp/profiles/` to map folder names to readable labels:

```json
{
  "Agonia.cz (3)": "Agonia Lite",
  "Agonia.cz (2)": "Agonia Full"
}
```

Edit this file to customize profile names in the dropdown.

## How It Works

1. Reads all `.jar` and `.jar.disabled` files from the mods directory
2. Extracts `fabric.mod.json` or `quilt.mod.json` from each JAR (ZIP archive)
3. Sanitizes malformed JSON (BOM, comments, trailing commas, raw newlines)
4. Falls back to regex extraction if JSON parsing fails
5. Saves a timestamped snapshot as JSON
6. Compares against the previous snapshot to detect changes
7. Generates a Markdown report

## License

MIT
