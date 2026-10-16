# Book Save verification

## Current tray lifecycle, 2026-10-15

The main window hides on **X**. It has no visible taskbar button while hidden. The tray menu has **Open Book Save** and **Exit**. Open restores the same window and keeps its editor state. A selected-text capture shows the window for review. **Exit** quits the app. Shutdown stops the global hook, waits for the save worker to finish, and removes the tray icon. Explorer's `TaskbarCreated` message adds the icon again after Explorer restarts. Tray startup failure shows the main window with an error; closing that window exits the app.

`verify-lifecycle.ps1` launches the debug app with temporary user data and a test-only mutex name. It checks hidden startup, second-launch restore, close-to-hide, tray Open, and tray Exit. It does not touch the clipboard or user saved data. `verify-clipboard.ps1` exercises selected-text copying, clipboard restoration, no-selection behavior, injected key rejection, and image cleanup. Its final result confirmed the original user clipboard bytes, formats, and image content were restored.

Validation on this change: `rustup run stable cargo check --locked --offline`, `rustup run stable cargo test --locked --offline` (10 passed), `rustup run stable cargo build --locked --offline`, `powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\verify-lifecycle.ps1` (all five lifecycle checks passed), and `powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\verify-clipboard.ps1` (all checks passed). These checks did not exercise a physical Left Shift gesture or the visible GPUI draft field. No install, package, or installed app files were changed.

Entries below record earlier app versions and verification runs.

## App icon and safe install/uninstall lifecycle, 2026-10-15

The Windows executable now embeds a clean **BS** monogram icon, visible on the exe, taskbar button, and both shortcuts. `assets/make-icon.ps1` renders the monogram with only built-in .NET `System.Drawing` (deep slate-blue rounded tile, white bold Segoe UI letters) into `assets/book-save.ico` as PNG-compressed ICO entries at 16, 32, 48, and 256 px (8,323 bytes). `assets/book-save.rc` plus `build.rs` link it through the Windows SDK resource compiler via the `embed-resource` Windows-only build dependency; no runtime dependency is added. Both shortcuts set `IconLocation` to the installed exe. The release exe grew from 24,208,384 to 24,217,088 bytes (+8,704, about 0.04%); `dumpbin` still reports `2 subsystem (Windows GUI)`.

`install.ps1` and `uninstall.ps1` no longer `Stop-Process -Force` a running app, which could interrupt the FIFO asynchronous save. Both now refuse with an exit code and tell the user to open Book Save and select **Quit** in the app header so pending saves finish, then run the script again. Startup background behavior is unchanged: the installer still creates the `Book Save Background.lnk` Startup shortcut with `--background`, and the app still minimizes on close with single-instance handover.

Validation on Windows: `cargo test --locked --offline` (10 passed), release build with no new warnings, rustfmt check on all six sources including `build.rs`, PowerShell parse of all seven scripts, and `git diff --check` passed. `ExtractAssociatedIcon` on the release and installed exes returns the 32 px BS monogram; the packaged exe in `dist/Book-Save-Windows-x64.zip` (8,537,891 bytes, rebuilt by `package.ps1`) carries it too, and both installed shortcuts report `IconLocation` of the installed exe. Live checks: clean install created both shortcuts and kept `captures.json` (SHA-256 unchanged across the whole round-trip); launching the installed app then running the installer exits 1 with the refuse message and leaves the process alive; a second launch exits 0 with no new PID; the uninstaller likewise refuses while running; `--background` starts with no main window and the next launch exits 0 and reveals it in the same PID; final uninstall removed the exe and both shortcuts while keeping the saved list, and a reinstall restored the working state. The two test-owned idle processes used for the refusal/background checks were stopped directly after their checks with no saves pending; user data was never at risk.

## Background lifecycle and console-free launch, 2026-10-15

The release executable now uses the Windows GUI subsystem (release only, via `windows_subsystem`; debug keeps its console), so launching from the Start menu opens no terminal window. `dumpbin` reports `2 subsystem (Windows GUI)` for the release exe.

Closing the window minimizes to the taskbar instead of quitting (`on_window_should_close` veto + `minimize_window`), so the process and its global keyboard hook stay alive. A **Quit** button in the header calls `remove_window` for a real exit. A second launch detects the named `Local\BookSaveSingleInstance` mutex, restores the running `Book Save` window with `SW_RESTORE`/`SetForegroundWindow`, and exits 0 without installing a second hook. Logon startup is a per-user Startup-folder shortcut (`Book Save Background.lnk`) launching with `--background`, which opens the window hidden (`show: false, focus: false`); the next Start-menu launch reveals it. The installer creates that shortcut and stops a running app instead of refusing; the uninstaller stops the app and removes both shortcuts. Both keep the saved list. `bench.ps1` stops the process directly since close no longer quits. Clipboard capture code is untouched.

Validation on Windows: `cargo check`/`cargo test` (10 passed, including the new background-flag test), release build, rustfmt check, PowerShell parse of all six scripts, and `git diff --check` passed. Live checks against the installed app: normal launch shows `Book Save`; a second launch exits 0 in ~275 ms with no new process; `CloseMainWindow` leaves the process alive and minimized (`IsIconic` true); another launch restores it in the same PID; `--background` starts with no main window and is revealed by the next launch. Install/uninstall round-trip preserved `captures.json` (SHA-256 unchanged) and removed both shortcuts plus the exe. `package.ps1` rebuilt the zip (24,208,384-byte exe). `verify-clipboard.ps1 -ScanOnly` passed (6 formats backed up, clipboard unchanged); the full verifier still fails at foregrounding its editor window, which is environmental (plain Notepad gets no main-window handle in this session) and unrelated to this change.

## Windows install and rename, 2026-10-15

The application crate and executable are named `book-save`. The Windows app folder is `book-save-app`. Git records the tracked source files as renames.

`package.ps1` created `dist/Book-Save-Windows-x64.zip` with the release executable and install scripts. The archive was extracted to a temporary folder and installed through its packaged `install.ps1`. The installer copied `book-save.exe` to `%LOCALAPPDATA%\Programs\Book Save`, created a Start menu shortcut, and preserved the existing user data file on a repeat install.

The first install copied the two existing list items from `data/captures.json` to `%LOCALAPPDATA%\Book Save\captures.json`. The app then started from its installed path and opened a window titled `Book Save`. Its release executable is 24,199,168 bytes. The package archive is 8,526,379 bytes.

All 9 app tests passed, including the new data migration test. The release build, Rust formatting check, PowerShell script parsing, and native clipboard verification passed after the rename. The end-user steps are in [INSTALL_WINDOWS.md](../docs/INSTALL_WINDOWS.md).

## Performance and architecture pass, 2026-10-15

Implemented event-driven delivery for capture and persistence events, removed the Windows clipboard worker's 50 ms idle polling by signaling its message thread, cached lowercase capture text for search, and made list filtering/counting a single pass without cloning capture strings during rendering. JSON writes now run on a FIFO background worker, write and flush a temporary file before replacement, and update the visible list only after a successful save. The UI disables conflicting edits while saving and queues a global capture until the save completes. The GPUI Kit icon bundle now includes only Check and Close.

Validation on Windows: `cargo test --locked --offline` passed all 9 tests; `cargo check --locked --offline` and optimized release build succeeded; rustfmt check and `git diff --check` passed. The native clipboard verifier exited 0 and passed all formats, no-selection restoration, private-format refusal, injected Shift rejection, 20 repeated image captures without GDI object growth, and exact clipboard restoration. The verifier does not generate a physical Shift gesture or drive the GPUI view; the physical gesture was previously exercised by the user.

The final release benchmark ran 7 launches with 12-second idle samples on an Intel i5-1235U (10 cores / 12 logical processors). All runs completed, `data/captures.json` SHA-256 was unchanged, and the release executable was 24,197,120 bytes. Warm startup median was 532.75 ms (range 513.63–689.18 ms, 6 runs); including the cold-ish first launch, median was 582.69 ms (513.63-1,589.42 ms). Working set median was 79.58 MiB (79.16–81.43); private memory median was 83.41 MiB (83.17–83.43). Idle CPU was 2.34% median over all runs and 2.405% over the six warm runs (0.65–2.73%).

Compared with the earlier 7-run baseline, startup and memory are within run variation. The executable is 10,752 bytes smaller (about 0.04%) than the 24,207,872-byte baseline. Idle CPU does not establish an improvement: the machine had active desktop processes, and the sample ranges overlap the baseline. Treat CPU and small startup differences as inconclusive. This harness measures window-handle readiness, process CPU and memory during idle, and executable size; it does not measure typing latency, large-list scaling, or physical gesture-to-field latency.

## Clipboard capture regression, 2026-10-15

The user's repeated failures progressed through HTML Format, `CF_BITMAP` (2), and `CF_DIB` (8). The read-only verification tool reproduced the last error on the actual Windows clipboard. Its eight formats included `CF_BITMAP`, PNG, `CF_DIB`, and `CF_DIBV5`. Supporting one representation left another representation of the same image unsupported.

The fix groups supported formats by their Windows storage type. A single match selects HGLOBAL byte copying, GDI bitmap copying, or logical palette copying. DIB, DIBV5, file lists, and the other documented memory formats now use the existing bounded memory path. Unsupported owner-managed formats still fail before any copy command.

The native editor check also exposed two runtime bugs. `CloseClipboard` synthesized text formats and advanced the marker sequence from 278 to 281. Sampling the sequence after close prevents that change from masquerading as a source copy. The clipboard owner thread also needed to process Windows messages while idle and during clipboard waits. Without message processing, the external editor held the clipboard while waiting for an ownership notification, and capture timed out.

Run `.\verify-clipboard.ps1` for the repeatable integration check. It compiles the production Windows module into a separate tool and copies selected Unicode text from a native editor in a different process. The original clipboard is backed up before fixtures and restored afterward. Comparisons cover the complete format set, raw memory bytes, bitmap dimensions and pixels, and palette entries. Output is saved at `.local\clipboard-verification.txt`.

The checks cover the actual user clipboard, plain text, HTML and RTF, bitmap, DIB, DIBV5, indexed DIB with a palette, file lists, and other standard memory formats. They also check no-selection restoration, unsupported private-format refusal without a sequence change, injected Shift rejection by the real global hook, and 20 repeated image captures without GDI object growth. The tool does not emulate a physical keyboard or test GPUI draft rendering. The previous user screenshots demonstrate that the physical gesture reaches the clipboard path; the gesture logic is unchanged by this fix.

The sections below record earlier Book Save sessions. Their unverified clipboard claims describe those earlier sessions.

The decision is whether a native GPUI Kit capture field and local card list can support the Book Save workflow. This artifact is throwaway. The user specified the compact dark direction, so no alternative layouts or stack substitutions were built.

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

That command exited with code 0 and installed `rustc 1.99.0 (b940084d7 2026-09-28)`. The manifest and README now require 1.99, the compiler used for this Book Save.

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

Each command printed `Running target\debug\book-save.exe`. The first and third runs closed with **Alt+F4** and exited with code 0. The fourth run was left open after verification.

Windows Computer Use supplied screenshots, the accessibility tree, pointer clicks, and keyboard input. These behaviors were observed in the real GUI:

- The window rendered a compact dark panel, a capture field, an Add button, search, filters, and cards.
- **Enter** saved `Read the Book Save capture notes`, cleared the capture field, and displayed a card.
- **Add** saved `Plan the compact capture panel` as the first card in the list.
- Completing the first capture changed its checkbox and the open count. Reading the JSON file confirmed `complete: true`.
- **Done** displayed only the completed card. **Open** displayed only the open card.
- **Edit**, text replacement, and **Enter** changed the second card to `Plan the Book Save capture panel`. The JSON file contained the edited text.
- **Ctrl+F** focused search. The uppercase query `PANEL` matched the edited card. With **All** selected, search displayed one of the two cards. Clearing search restored both cards.
- **Ctrl+N** focused capture. An unsaved draft, `Draft kept during editing`, returned after opening an edit and canceling with **Escape**.
- **Delete** removed the first card. Reading the JSON confirmed that its ID was absent.
- Completing the remaining edited capture and restarting restored its text and checked completion state. The deleted card stayed absent. Two subsequent relaunches showed this saved state.

The final JSON contains one demonstration capture with ID 2, text `Plan the Book Save capture panel`, and `complete: true`. The unsaved draft was not persisted. The final restart displayed `1 captures loaded` and `0 open`.

The observed restored window is saved at [.local/restarted-window.png](.local/restarted-window.png). Its accessibility tree is in [.local/restarted-accessibility.json](.local/restarted-accessibility.json).

## Limits and runtime anomaly

The first relaunch exited with `0xffffffff` and no diagnostic beyond Cargo's exit report. Its accessibility tree briefly exposed the restored completed card, but its screenshot captured background content. Activation then failed with `foreground window did not report a process id`, and the process had exited. No matching Application event was found for the executable. The cause is unconfirmed. The next two relaunches restored the data and rendered correctly. This is an unresolved runtime anomaly, not a successful stability check.

Windows UI Automation exposed editable fields, buttons, and checkboxes. It reported the focused element as the root region, even while the input showed a caret and accepted typing. Card text did not appear in that tree. Screen reader support is not verified.

No automated tests, large-list stress checks, save-failure simulations, or performance measurements were run during the original Book Save session. Tray behavior remains outside the Book Save scope. Global capture verification is recorded separately below.

The observed workflow supports continuing a GPUI Kit evaluation for this capture panel. Investigate the unexplained relaunch exit and accessibility limitations before making a production decision.

## Files created

Authored project files are `src/main.rs`, `Cargo.toml`, `.cargo/config.toml`, `run.ps1`, `README.md`, `VERIFICATION.md`, and `.gitignore`. Cargo generated `Cargo.lock`. Build tools generated `.local` and `target`. Native interaction generated `data/captures.json`, the screenshot, and the accessibility record. All task artifacts are inside `book-save-app`.

## Design choices

One GPUI entity owns the capture list and retained input states. The composer uses a `Capture` or `Edit` enum, which preserves the capture draft during editing. This applies the Model the Domain principle.

The verification distinguishes a successful build from observed GUI behavior. This applies the Prove It Works principle.

The implementation delegate worked in-thread because the workspace was empty, the Book Save was small, and the user required one isolated directory. No production architecture, git repository, PR, or tests were created.

## Windows global capture implementation

This change starts from `5e68e67` in the isolated worktree at `C:\Users\ACER\.t3\worktrees\book-save\t3-ea2f0239`. The earlier interaction results above describe the original Book Save, not verification of this change.

`CaptureEvent` carries readiness, selected Unicode text, or a visible failure from the OS worker. GPUI polls the channel every 50 milliseconds in a retained UI-runtime task. Selected text fills the existing input without saving. An edit or load error prevents delivery. Persistence and CRUD methods keep their existing behavior.

The two candidate OS designs were a `WH_KEYBOARD_LL` callback on a dedicated message-loop thread and raw input through a hidden window. The callback was chosen for the explicit injected-input flags and the small integration boundary. It only detects taps and queues a request. A separate clipboard worker owns the temporary clipboard transaction. Raw input would require device registration and another window procedure. Microsoft recommends raw input for general asynchronous monitoring, but this feature needs the hook's explicit injected-input distinction. See [LowLevelKeyboardProc](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc).

The hook uses `ShiftGesture` to track released, pressed, and triggered states. It recognizes distinct Left Shift key-down events at most 350 milliseconds apart and queues the copy on the second key-up. This avoids racing the asynchronous Shift state update. Repeats and injected events do not advance the gesture. Other physical key-down events cancel the pending tap. Other held modifiers block the pair. Requests do not overlap.

Before changing the clipboard, the worker duplicates every supported format into owned memory. It supports an empty clipboard and `CF_TEXT`, `CF_OEMTEXT`, `CF_UNICODETEXT`, and `CF_LOCALE`. It also preserves registered formats in the inclusive ID range `0xC000` through `0xFFFF`, including HTML and RTF. Windows requires registered clipboard data to use HGLOBAL handles. See [RegisterClipboardFormatA](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerclipboardformata). Each registered format must have a nonzero `GlobalSize` of at most 16 MiB and pass `GlobalLock` before copying. `CF_BITMAP` is size-checked from its bitmap row size and height, duplicated with `CopyImage`, and restored with `SetClipboardData`. Unsupported predefined formats and data that fails the size or memory checks cause a refusal before any copy command. The worker uses a message-only owner window, since clipboard restoration needs a valid owner. See [SetClipboardData](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setclipboarddata) and [CopyImage](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-copyimage).

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
3. Put HTML or RTF on the clipboard with all formats within the backup bounds. Confirm capture succeeds and restores the original formats. Put a bitmap on the clipboard and confirm the original image returns after capture. Put an unsupported predefined format such as file data, or a format larger than 16 MiB. Confirm capture refuses without changing the clipboard. Try clipboard contention and a source app running elevated.
4. Capture while editing a card. Confirm the edit remains intact. Close Book Save during a pending capture and confirm the process exits and subsequent taps do nothing.

An injected-input check can verify rejection. It is not evidence of physical gesture success or of successful clipboard rollback.

### Results from this implementation session

Verification ran on Windows with the isolated Rust 1.99 stable toolchain. Cargo used the existing dependency cache at `C:\Users\ACER\Documents\Endeavor\hobby\book-save\book-save-app\.local\cargo`. Build output stayed in this worktree's `book-save-app\target`. CMake came from the Visual Studio 2022 Community directory documented above.

These commands completed successfully in this worktree's `book-save-app` directory:

```powershell
rustup run stable cargo check --offline
rustup run stable cargo build --locked --offline
& 'C:\Users\ACER\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\rustfmt.exe' --check --edition 2024 src/main.rs src/global_capture.rs src/global_capture_windows.rs
git diff --check
& '.\target\debug\book-save.exe'
```

`CARGO_HOME` and `RUSTUP_HOME` pointed to the existing Book Save's `.local\cargo` and `.local\rustup` for the Cargo commands. The build passed again after the key-up handoff change. The isolated toolchain lacked rustfmt, so formatting used the existing installed rustfmt executable shown above.

Computer Use drove the actual built executable with injected input. The observed results were:

- Startup displayed `Select text in another app, then double-tap Left Shift.` This confirms that hook installation reported success and the typed readiness event reached GPUI. It does not prove physical gesture detection.
- Two injected Left Shift calls completed within 115 milliseconds. The capture field stayed empty and the ready status remained unchanged. This is a negative injected-input check in Book Save itself, not a selected-text capture from another app.
- Enter saved `Windows capture smoke check`. The card appeared, and the JSON contained that text with `complete: false`.
- Edit and Save changed the text to `Windows capture smoke check edited`. The JSON matched the displayed card.
- Completing the card changed the JSON to `complete: true`. Closing with Alt+F4 exited with code 0.
- Restart restored the edited text and completed state. Delete removed this session's demonstration card, and the JSON became `[]`.
- The second Alt+F4 shutdown also exited with code 0. A process query found no remaining `book-save` process.

The observed running window is saved at [.local/global-capture-window.png](.local/global-capture-window.png). Its accessibility tree is saved at [.local/global-capture-accessibility.txt](.local/global-capture-accessibility.txt). These files and the empty demonstration data file are untracked verification artifacts in this worktree. The first screenshot returned background content before activation. Explicit activation returned the actual Book Save window. This screenshot anomaly does not establish an app crash.

Physical gesture success, successful OS selection capture, clipboard restoration, clipboard failure paths, and shutdown during an active copy remain unverified. Filters, search, edit cancellation, accessibility support, and other platforms were not rechecked in this session. No automated tests were added or run. A narrow delegated comment audit reported zero comments to delete and no suppression attributes. The coordinating parent owns broader correctness review.

### Registered clipboard format fix

The fix starts from `dccafc2` in `C:\Users\ACER\.t3\worktrees\book-save\t3-8d3805c4`. It extends only the backup format check to include registered IDs. The bounded HGLOBAL copy, clipboard sequence and owner checks, and restoration logic remain unchanged.

These commands exited with code 0 in this worktree's `book-save-app` directory:

```powershell
& 'C:\Users\ACER\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\rustfmt.exe' --edition 2024 src/main.rs src/global_capture.rs src/global_capture_windows.rs
& 'C:\Users\ACER\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\rustfmt.exe' --check --edition 2024 src/main.rs src/global_capture.rs src/global_capture_windows.rs
rustup run stable cargo build --locked --offline
```

The Windows build used the existing Book Save's `.local\cargo` and `.local\rustup` directories and the Visual Studio CMake directory documented above. Build output stayed in this worktree's `book-save-app\target`. No automated tests or runtime clipboard checks were run for this fix. The coordinating parent owns review and runtime verification.

### Tray lifecycle verification

`verify-lifecycle.ps1` launches only test instances. Each debug process gets a unique mutex identity and a temporary `LOCALAPPDATA` folder. The checks do not read or modify the installed app's saved list or the clipboard. The verifier scopes native tray menus to its child PID and clicks relative to the actual menu rectangle because Windows exposes the native popup without UI Automation menu items in this environment.

The run completed with these checks passing: hidden startup with a tray host, repeated-launch handover to the same instance, close-to-hide while the process stays alive, tray Open, unsaved capture-field draft retention across hide/restore, a targeted `TaskbarCreated` message to the test-owned tray window, tray Exit, and both injected startup and runtime tray-failure fallbacks. The “process stays alive” check does not establish that a physical capture gesture works.

Three ten-second hidden-process samples from the unoptimized debug build measured CPU at 0.47%, 0.62%, and 0.00% of one core; average private bytes were 92.6, 86.9, and 86.9 MiB; average working set was 108.1, 107.2, and 107.2 MiB. These are direct Windows process counters with no memory trim. They describe this debug verification process only; they are not release-build measurements.

The draft check uses UI Automation's `ValuePattern` to set and read the actual GPUI capture field. Tray startup failure injection and tray re-add failure injection are debug-only environment switches. The Explorer simulation posts the registered message directly to the test process's tray window; it does not restart Explorer. Physical keyboard capture remains unverified.
