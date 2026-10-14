# Prototype verification

The decision is whether a native GPUI Kit capture field and local card list can support the Cooper workflow. This artifact is throwaway. The user specified the compact dark direction, so no alternative layouts or stack substitutions were built.

## Prerequisites observed

- `rustc --version` printed `rustc 1.91.1 (ed61e7d7e 2025-11-07)` for the existing toolchain.
- `cargo --version` printed `cargo 1.91.1 (ea2d97820 2025-10-10)`.
- `rustup show active-toolchain` printed `stable-x86_64-pc-windows-msvc (default)`.
- `vswhere.exe -latest -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath` found Visual Studio 2022 Community.
- Windows SDK library directories `10.0.22000.0` and `10.0.22621.0` exist.
- CMake exists at `C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe`. It was absent from the initial `PATH`.

An isolated install used this command with `RUSTUP_HOME=.local\rustup`, `CARGO_HOME=.local\cargo`, and both `TEMP` and `TMP` set to `.local\tmp`:

```powershell
rustup toolchain install 1.92.0-x86_64-pc-windows-msvc --profile minimal
```

Rustup installed Cargo, the standard library, and rustc. It then exited with code 1 during its separate self-update check because rustup itself was not installed in the isolated Cargo directory. The installed toolchain was checked directly with the same local environment:

```powershell
rustup run 1.92.0-x86_64-pc-windows-msvc rustc --version
```

That command exited with code 0 and printed `rustc 1.92.0 (ded5c06cf 2025-12-08)`.

The Rust 1.92 build failed inside `gpui-pre-util 0.3.9` at `src/lib.rs:575` with `E0658`, use of unstable library feature `slice_as_array`. The stack and dependency version were kept. The isolated compiler was upgraded with the same local environment:

```powershell
rustup toolchain install stable-x86_64-pc-windows-msvc --profile minimal --no-self-update
```

That command exited with code 0 and installed `rustc 1.99.0 (b940084d7 2026-09-28)`. The manifest and README now require 1.99, the compiler used for this prototype.

## Build and native interaction

The native project built successfully and the real window was driven through the capture workflow. No tests were added.

Dependency fetching used the existing Cargo executable with `CARGO_HOME=.local\cargo`, `TEMP=.local\tmp`, and `TMP=.local\tmp`:

```powershell
& 'C:\Users\ACER\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\cargo.exe' fetch
```

It exited with code 0 and generated `Cargo.lock`. The lock contains GPUI Kit 0.7.3 and its pinned GPUI 0.3.9 snapshot.

Build progress is captured with:

```powershell
.\run.ps1 -Command build 2>&1 | Tee-Object -FilePath .local\build.log
```

The first attempt exposed Windows PowerShell treating native stderr progress as an error during capture. The launcher now converts native output to strings and preserves Cargo's exit code. Compilation is limited to two jobs through `.cargo\config.toml`.

The final build exited with code 0 and printed `Finished dev profile [unoptimized]`. Its output is in `.local\build.log`.

The actual launch commands were:

```powershell
.\run.ps1 2>&1 | Tee-Object -FilePath .local\run.log
.\run.ps1 2>&1 | Tee-Object -FilePath .local\restart.log
.\run.ps1 2>&1 | Tee-Object -FilePath .local\restart-second.log
.\run.ps1 2>&1 | Tee-Object -FilePath .local\restart-final.log
```

Each command printed `Running target\debug\cooper-gpui-prototype.exe`. The first and third runs closed with **Alt+F4** and exited with code 0. The fourth run was left open after verification.

Windows Computer Use supplied screenshots, the accessibility tree, pointer clicks, and keyboard input. These behaviors were observed in the real GUI:

- The window rendered a compact dark panel, a capture field, an Add button, search, filters, and cards.
- **Enter** saved `Read the Cooper capture notes`, cleared the capture field, and displayed a card.
- **Add** saved `Plan the compact capture panel` as the first card in the list.
- Completing the first capture changed its checkbox and the open count. Reading the JSON file confirmed `complete: true`.
- **Done** displayed only the completed card. **Open** displayed only the open card.
- **Edit**, text replacement, and **Enter** changed the second card to `Plan the Cooper capture panel`. The JSON file contained the edited text.
- **Ctrl+F** focused search. The uppercase query `PANEL` matched the edited card. With **All** selected, search displayed one of the two cards. Clearing search restored both cards.
- **Ctrl+N** focused capture. An unsaved draft, `Draft kept during editing`, returned after opening an edit and canceling with **Escape**.
- **Delete** removed the first card. Reading the JSON confirmed that its ID was absent.
- Completing the remaining edited capture and restarting restored its text and checked completion state. The deleted card stayed absent. Two subsequent relaunches showed this saved state.

The final JSON contains one demonstration capture with ID 2, text `Plan the Cooper capture panel`, and `complete: true`. The unsaved draft was not persisted. The final restart displayed `1 captures loaded` and `0 open`.

The observed restored window is saved at [.local/restarted-window.png](.local/restarted-window.png). Its accessibility tree is in [.local/restarted-accessibility.json](.local/restarted-accessibility.json).

## Limits and runtime anomaly

The first relaunch exited with `0xffffffff` and no diagnostic beyond Cargo's exit report. Its accessibility tree briefly exposed the restored completed card, but its screenshot captured background content. Activation then failed with `foreground window did not report a process id`, and the process had exited. No matching Application event was found for the executable. The cause is unconfirmed. The next two relaunches restored the data and rendered correctly. This is an unresolved runtime anomaly, not a successful stability check.

Windows UI Automation exposed editable fields, buttons, and checkboxes. It reported the focused element as the root region, even while the input showed a caret and accepted typing. Card text did not appear in that tree. Screen reader support is not verified.

No automated tests, large-list stress checks, save-failure simulations, or performance measurements were run. Global capture and tray behavior are outside the prototype scope.

The observed workflow supports continuing a GPUI Kit evaluation for this capture panel. Investigate the unexplained relaunch exit and accessibility limitations before making a production decision.

## Files created

Authored project files are `src/main.rs`, `Cargo.toml`, `.cargo/config.toml`, `run.ps1`, `README.md`, `VERIFICATION.md`, and `.gitignore`. Cargo generated `Cargo.lock`. Build tools generated `.local` and `target`. Native interaction generated `data/captures.json`, the screenshot, and the accessibility record. All task artifacts are inside `prototype-gpui`.

## Design choices

One GPUI entity owns the capture list and retained input states. The composer uses a `Capture` or `Edit` enum, which preserves the capture draft during editing. This applies the Model the Domain principle.

The verification distinguishes a successful build from observed GUI behavior. This applies the Prove It Works principle.

The implementation delegate worked in-thread because the workspace was empty, the prototype was small, and the user required one isolated directory. No production architecture, git repository, PR, or tests were created.
