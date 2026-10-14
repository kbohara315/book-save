# Run the Cooper GPUI prototype

This throwaway native Windows app explores whether GPUI Kit can support a compact capture panel and a persistent local list. It is isolated from production code.

## Install the Windows prerequisites

Use Windows 10 or later, Rust 1.99 or later with the MSVC target, and Visual Studio 2022 Build Tools or Community. Install the **Desktop development with C++** workload with MSVC and a Windows SDK. CMake must be available in the build terminal. The launcher also finds CMake bundled with Visual Studio.

The documentation gives a Rust 1.92 baseline, but the actual GPUI snapshot failed to compile on 1.92 with an unstable `slice_as_array` error. This prototype uses the current stable Rust toolchain, 1.99.

The setup follows the current [GPUI Kit installation guide](https://gpui-kit.com/docs/installation/) and [getting started guide](https://gpui-kit.com/docs/getting-started/). The manifest pins GPUI Kit 0.7.3 with its default components and icon assets. It does not enable `gpui-fast`.

## Start the app

Run these commands in PowerShell:

```powershell
Set-Location 'C:\Users\ACER\Documents\Endeavor\hobby\book-save\prototype-gpui'
.\run.ps1
```

If PowerShell blocks the script under its current execution policy, run it in a new process:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File 'C:\Users\ACER\Documents\Endeavor\hobby\book-save\prototype-gpui\run.ps1'
```

The underlying command is `cargo run --locked`. To build without launching, use `.\run.ps1 -Command build`. The first build downloads and compiles the framework dependencies.

The launcher keeps Cargo downloads, temporary build files, and build output under this directory. It uses the local stable Rust toolchain when `.local\rustup` contains that toolchain. Otherwise, it uses your installed Rust toolchain. It restores the process environment when it exits.

## Try the workflow

1. Type into the capture field and press **Enter**, or click **Add**. Blank captures are ignored. New cards appear first.
2. Toggle **Mark complete** on a card. Switch between **All**, **Open**, and **Done**.
3. Type into **Search captures** to filter the current view by text. Search ignores letter case.
4. Click **Edit**. Change the text in the top field, then press **Enter** or click **Save**. **Escape** or **Cancel** restores the previous capture draft.
5. Click **Delete** to remove a card immediately.
6. Close the window and restart the app. Saved captures and completion states reload.

**Ctrl+N** focuses the capture field. **Ctrl+F** focuses search. Adding a capture clears the search and selects **All** so the new card is visible. Editing preserves the current filter and search.

## Capture selected text on Windows

1. Leave Cooper running and select text in another app that supports **Ctrl+C**.
2. Press and release **Left Shift** twice. The two key-down events must be at most 350 milliseconds apart. Release the second press promptly.
3. Cooper fills its capture field and requests window activation. Review the text, then press **Enter** or click **Add** to save.

The capture replaces the current unsaved capture draft. An active edit or a load error blocks delivery and leaves the existing field untouched. Capture failures appear below the list. No text is saved automatically.

The shortcut ignores key repeats, injected input, Right Shift, and gestures with another modifier held. Another key press cancels the pending tap. Captures do not overlap. UI automation sends injected keystrokes, so it cannot prove that the physical gesture works.

Cooper temporarily sends **Ctrl+C** to the foreground app and reads fresh Unicode clipboard text. It waits up to 800 milliseconds for a copy. It restores the original clipboard after success or an unusable selection. An empty clipboard is supported. The backup copies these formats according to their storage type:

- HGLOBAL memory for ANSI text, OEM text, Unicode text, locale data, SYLK, DIF, TIFF, DIB, DIBV5, RIFF, WAVE, file lists (`CF_HDROP`), and display text (`CF_DSPTEXT`). Registered formats such as HTML, RTF, and PNG use the same memory backup.
- A copied GDI bitmap for `CF_BITMAP` and `CF_DSPBITMAP`.
- A copied logical palette for `CF_PALETTE`.

Each format must be nonempty and at most 16 MiB. Owner-display, pen data, metafile formats, reserved formats, and private handle formats cause a visible refusal before the clipboard changes. Data that exceeds the limit or cannot be copied safely also causes a refusal. Copy plain text or clear the clipboard before retrying after a refusal.

If another process takes ownership of the clipboard during capture, Cooper cancels and leaves that process's clipboard alone. Clipboard access or restoration can fail if another app holds the clipboard. These failures appear in the app. Clipboard history and clipboard managers may observe the temporary copy. An app that completes its copy after the timeout can still change the clipboard later.

This is a Windows desktop hook, not a registered system hotkey. The source app must support copying selected text as Unicode with **Ctrl+C**. Some editors copy a whole line when no text is selected. Elevated apps, secure desktops, unusual clipboard owners, and apps with custom copy commands may not work. Windows can silently remove a hook that times out. Cooper releases the hook and joins both OS threads on normal shutdown. Other platforms use a no-op worker and show an unsupported message while retaining manual capture.

## Keep the data local

The app saves a JSON array to `data\captures.json` inside this project. Each capture has `id`, `text`, and `complete`. Every mutation writes a temporary file, flushes it, and replaces the previous JSON file before updating the list. A failed save leaves the draft and previous list available.

If the file cannot load, the app displays the error and disables mutations to protect the existing file. Repair the file and restart. Run one app process at a time. This prototype has no file locking or multi-process merge behavior.

The prototype has no tray, image import, sections, or sync. **Ctrl+N** and **Ctrl+F** apply only while its window is focused.

## Benchmark the Windows build

Run `.\bench.ps1` from this directory after building the release executable:

```powershell
$env:CARGO_HOME = '.local\cargo'
$env:RUSTUP_HOME = '.local\rustup'
$env:CARGO_TARGET_DIR = 'target'
rustup run stable-x86_64-pc-windows-msvc cargo build --release --locked
.\bench.ps1 -Runs 7 -IdleSeconds 12
```

The benchmark launches Cooper seven times, waits for its main window handle, samples process CPU and memory during each 12-second idle period, and records executable size. The first run is marked cold-ish and later runs warm. CSV output goes to `.local\bench-results.csv`. The script refuses to run when another Cooper process is open and checks that `data\captures.json` is unchanged.

## Verify Windows clipboard capture

Run `.\verify-clipboard.ps1` to check the production capture code against a separate native editor with selected Unicode text. The check preserves the current clipboard, exercises image, palette, text, rich-text, and file-list formats, and compares the restored bytes and pixels. It also checks unsupported-format refusal, no-selection restoration, injected-key rejection, and repeated capture for GDI handle leaks. A temporary editor window appears during the check. Results are saved to `.local\clipboard-verification.txt`.

Run `.\verify-clipboard.ps1 -ScanOnly` to list formats and check backup without changing the clipboard. The verification crate includes the production Windows module directly. It adds no dependency to the shipped app. These checks exercise actual Windows copying and restoration. They do not generate physical keyboard events.

Read [VERIFICATION.md](VERIFICATION.md) for commands, observed results, and remaining limits. The native workflow ran successfully, including save and reload. One relaunch exited unexpectedly without a diagnostic. Two later relaunches worked. Treat the runtime anomaly as unresolved.
