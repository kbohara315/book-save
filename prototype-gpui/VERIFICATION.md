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

No automated tests, large-list stress checks, save-failure simulations, or performance measurements were run during the original prototype session. Tray behavior remains outside the prototype scope. Global capture verification is recorded separately below.

The observed workflow supports continuing a GPUI Kit evaluation for this capture panel. Investigate the unexplained relaunch exit and accessibility limitations before making a production decision.

## Files created

Authored project files are `src/main.rs`, `Cargo.toml`, `.cargo/config.toml`, `run.ps1`, `README.md`, `VERIFICATION.md`, and `.gitignore`. Cargo generated `Cargo.lock`. Build tools generated `.local` and `target`. Native interaction generated `data/captures.json`, the screenshot, and the accessibility record. All task artifacts are inside `prototype-gpui`.

## Design choices

One GPUI entity owns the capture list and retained input states. The composer uses a `Capture` or `Edit` enum, which preserves the capture draft during editing. This applies the Model the Domain principle.

The verification distinguishes a successful build from observed GUI behavior. This applies the Prove It Works principle.

The implementation delegate worked in-thread because the workspace was empty, the prototype was small, and the user required one isolated directory. No production architecture, git repository, PR, or tests were created.

## Windows global capture implementation

This change starts from `5e68e67` in the isolated worktree at `C:\Users\ACER\.t3\worktrees\book-save\t3-ea2f0239`. The earlier interaction results above describe the original prototype, not verification of this change.

`CaptureEvent` carries readiness, selected Unicode text, or a visible failure from the OS worker. GPUI polls the channel every 50 milliseconds in a retained UI-runtime task. Selected text fills the existing input without saving. An edit or load error prevents delivery. Persistence and CRUD methods keep their existing behavior.

The two candidate OS designs were a `WH_KEYBOARD_LL` callback on a dedicated message-loop thread and raw input through a hidden window. The callback was chosen for the explicit injected-input flags and the small integration boundary. It only detects taps and queues a request. A separate clipboard worker owns the temporary clipboard transaction. Raw input would require device registration and another window procedure. Microsoft recommends raw input for general asynchronous monitoring, but this feature needs the hook's explicit injected-input distinction. See [LowLevelKeyboardProc](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc).

The hook uses `ShiftGesture` to track released, pressed, and triggered states. It recognizes distinct Left Shift key-down events at most 350 milliseconds apart and queues the copy on the second key-up. This avoids racing the asynchronous Shift state update. Repeats and injected events do not advance the gesture. Other physical key-down events cancel the pending tap. Other held modifiers block the pair. Requests do not overlap.

Before changing the clipboard, the worker duplicates every supported format into owned movable memory. It supports an empty clipboard and `CF_TEXT`, `CF_OEMTEXT`, `CF_UNICODETEXT`, and `CF_LOCALE`. It also preserves registered formats in the inclusive ID range `0xC000` through `0xFFFF`, including HTML and RTF. Windows requires registered clipboard data to use HGLOBAL handles. See [RegisterClipboardFormatA](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerclipboardformata). Each format must have a nonzero `GlobalSize` of at most 16 MiB and pass `GlobalLock` before copying. Unsupported predefined formats, including non-HGLOBAL formats, and data that fails these bounds or memory checks cause a refusal before any copy command. The worker uses a message-only owner window, since clipboard restoration needs a valid owner. See [SetClipboardData](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setclipboarddata).

An empty Unicode marker separates a fresh copy from stale clipboard text. The worker waits up to 800 milliseconds for a changed clipboard sequence and accepts Unicode text only from a clipboard owner in the source process. It restores the original formats after success or no usable selection. A changed sequence after reading cancels restoration, preserving the newer clipboard. Clipboard contention or failed restoration produces an error. Clipboard history may retain the temporary contents. A source app that copies after the timeout may replace the clipboard later.

Normal app quit stops the worker, posts `WM_QUIT`, explicitly unhooks, and joins the hook and clipboard threads. Dropping the worker also stops it. Non-Windows builds select a no-op worker that reports unsupported global capture. A non-Windows target was not available on this machine, so cross-platform compilation was not run.

The implementation throughput checkpoint was:

- Blocking first steps were the full agent instructions, existing UI and persistence reads, and the OS ownership sketch.
- Independent workstreams were not used because this is one feature with one implementation owner.
- Shared mutable state stays inside one clipboard worker. The callback owns its tap state, and GPUI receives owned events through a channel.
- The smallest safe decomposition is one Windows module and one cross-platform event boundary. Broader review belongs to the coordinating parent.

Model the Domain shaped the `CaptureEvent` enum and the single owner for each kind of state. Prove It Works requires separate evidence for compilation, real UI behavior, and a physical gesture. No automated tests were added.

### Physical checks still required

Use a real keyboard with a source app such as Notepad. These checks have not been run by the implementation delegate:

1. Select Unicode text in the source app, double-tap Left Shift within 350 milliseconds, and release. Confirm one draft arrives, nothing saves automatically, and the original clipboard returns.
2. Try no selection, whitespace-only text, a held Shift, Right Shift, key repeats, another modifier held, three rapid taps, and a pair more than 350 milliseconds apart. Confirm only a valid pair captures once. Account for editors that copy a whole line without a selection.
3. Put HTML or RTF on the clipboard with all formats within the backup bounds. Confirm capture succeeds and restores the original formats. Put an unsupported predefined format such as a bitmap on the clipboard, or a format larger than 16 MiB. Confirm capture refuses without changing the clipboard. Try clipboard contention and a source app running elevated.
4. Capture while editing a card. Confirm the edit remains intact. Close Cooper during a pending capture and confirm the process exits and subsequent taps do nothing.

An injected-input check can verify rejection. It is not evidence of physical gesture success or of successful clipboard rollback.

### Results from this implementation session

Verification ran on Windows with the isolated Rust 1.99 stable toolchain. Cargo used the existing dependency cache at `C:\Users\ACER\Documents\Endeavor\hobby\book-save\prototype-gpui\.local\cargo`. Build output stayed in this worktree's `prototype-gpui\target`. CMake came from the Visual Studio 2022 Community directory documented above.

These commands completed successfully in this worktree's `prototype-gpui` directory:

```powershell
rustup run stable cargo check --offline
rustup run stable cargo build --locked --offline
& 'C:\Users\ACER\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\rustfmt.exe' --check --edition 2024 src/main.rs src/global_capture.rs src/global_capture_windows.rs
git diff --check
& '.\target\debug\cooper-gpui-prototype.exe'
```

`CARGO_HOME` and `RUSTUP_HOME` pointed to the existing prototype's `.local\cargo` and `.local\rustup` for the Cargo commands. The build passed again after the key-up handoff change. The isolated toolchain lacked rustfmt, so formatting used the existing installed rustfmt executable shown above.

Computer Use drove the actual built executable with injected input. The observed results were:

- Startup displayed `Select text in another app, then double-tap Left Shift.` This confirms that hook installation reported success and the typed readiness event reached GPUI. It does not prove physical gesture detection.
- Two injected Left Shift calls completed within 115 milliseconds. The capture field stayed empty and the ready status remained unchanged. This is a negative injected-input check in Cooper itself, not a selected-text capture from another app.
- Enter saved `Windows capture smoke check`. The card appeared, and the JSON contained that text with `complete: false`.
- Edit and Save changed the text to `Windows capture smoke check edited`. The JSON matched the displayed card.
- Completing the card changed the JSON to `complete: true`. Closing with Alt+F4 exited with code 0.
- Restart restored the edited text and completed state. Delete removed this session's demonstration card, and the JSON became `[]`.
- The second Alt+F4 shutdown also exited with code 0. A process query found no remaining `cooper-gpui-prototype` process.

The observed running window is saved at [.local/global-capture-window.png](.local/global-capture-window.png). Its accessibility tree is saved at [.local/global-capture-accessibility.txt](.local/global-capture-accessibility.txt). These files and the empty demonstration data file are untracked verification artifacts in this worktree. The first screenshot returned background content before activation. Explicit activation returned the actual Cooper window. This screenshot anomaly does not establish an app crash.

Physical gesture success, successful OS selection capture, clipboard restoration, clipboard failure paths, and shutdown during an active copy remain unverified. Filters, search, edit cancellation, accessibility support, and other platforms were not rechecked in this session. No automated tests were added or run. A narrow delegated comment audit reported zero comments to delete and no suppression attributes. The coordinating parent owns broader correctness review.

### Registered clipboard format fix

The fix starts from `dccafc2` in `C:\Users\ACER\.t3\worktrees\book-save\t3-8d3805c4`. It extends only the backup format check to include registered IDs. The bounded HGLOBAL copy, clipboard sequence and owner checks, and restoration logic remain unchanged.

These commands exited with code 0 in this worktree's `prototype-gpui` directory:

```powershell
& 'C:\Users\ACER\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\rustfmt.exe' --edition 2024 src/main.rs src/global_capture.rs src/global_capture_windows.rs
& 'C:\Users\ACER\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\rustfmt.exe' --check --edition 2024 src/main.rs src/global_capture.rs src/global_capture_windows.rs
rustup run stable cargo build --locked --offline
```

The Windows build used the existing prototype's `.local\cargo` and `.local\rustup` directories and the Visual Studio CMake directory documented above. Build output stayed in this worktree's `prototype-gpui\target`. No automated tests or runtime clipboard checks were run for this fix. The coordinating parent owns review and runtime verification.
