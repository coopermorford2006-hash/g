# Fortnite Ring: handoff to a session on the player's PC

This project was started in a cloud session that could not run either game. Continue it on the
Windows PC that has Elden Ring and Fortnite installed.

## State (branch `claude/fortnite-elden-ring-mashup-enrou5`)
- `sheets/*.json` are the source of truth; `python tools/preflight.py` must be clean before any build,
  then `python tools/codegen.py` regenerates `fortring/src/generated.rs` and `extractor/Generated.cs`.
- `fortring/`: the Elden Ring DLL (Rust, loaded by ModEngine2 through `external_dlls`). Compiles; 14 unit
  tests pass under Wine. Never run in the real game yet.
- `extractor/`: FortniteRingSetup.exe (C#, CUE4Parse). Keys/mappings/Oodle download verified; the asset
  export has never run against a real Fortnite install.
- Preflight currently fails on purpose: the 3D renderer (Fortnite outfit, guns, chests, build pieces,
  animations) is not written yet, so those sheet columns are unread.

## First tests on the PC
1. Build: `cargo build --release` in `fortring/` (MSVC toolchain), `dotnet publish -c Release` in `extractor/`.
2. Run the setup tool against the real Fortnite folder; read `%LOCALAPPDATA%\FortniteRing\setup.log` and
   `cache\manifest.json`, then fix the `query` cells in `sheets/fortnite_assets.json` with the real paths.
3. Start Elden Ring through ModEngine2 (offline, never the normal launcher) with `fortring.dll` in a
   config's `external_dlls`, on a new character. Read `%LOCALAPPDATA%\FortniteRing\FortniteRing.log`:
   boot, er_rows picks, camera basis, ui state ids, harvest hits, loot rolls.
4. Every cell listed as UNVERIFIED by the preflight gets checked in game and the sheet updated.

## Melty
Publishing uses the Melty MCP server. Its token is NOT stored in this repo: copy a fresh Publish prompt
from Melty Studio into the new session.
