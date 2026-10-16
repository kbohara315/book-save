# Install Book Save on Windows

Book Save installs for one Windows user. It does not need administrator access.

## Install from a ready-to-use package

1. Download `Book-Save-Windows-x64.zip` from the source that provided Book Save.
2. Extract all files from the zip to a folder.
3. Open PowerShell in that folder.
4. Run this command:

   ```powershell
   powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\install.ps1
   ```

The installer copies the app to `%LOCALAPPDATA%\Programs\Book Save`. It adds **Book Save** to the Start menu. It also adds **Book Save Background** to your personal Startup folder, so global capture keeps working after sign-in. Open it from the Start menu to run the app. The app shows a **BS** monogram icon in the Start menu, tray, and Startup shortcuts.

If Book Save is already running, the installer refuses to continue instead of closing the app for you. Open the tray menu and select **Exit**. Wait for Book Save to close, then run the installer again.

## Install from the source folder

Use this method when you do not have a ready-to-use package. Install Rust 1.99 or later with the MSVC target. Install Visual Studio 2022 Build Tools with the **Desktop development with C++** workload and a Windows SDK. Make CMake available in PowerShell. The app launcher can find the CMake copy inside Visual Studio.

Open PowerShell in the repository folder and run:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\book-save-app\install.ps1
```

The installer builds the release app, installs it, and creates the Start menu shortcut. The first build needs an internet connection to download Rust packages.

To make a package for another Windows user, build the release zip from the repository folder:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\book-save-app\package.ps1
```

The zip is written to `book-save-app\dist\Book-Save-Windows-x64.zip`.

## Running in the background

The global keyboard hook only works while Book Save runs, so the app stays running after you close its window:

- Opening Book Save from the Start menu shows its window. No terminal window opens.
- Book Save starts hidden each time you sign in. Double-tap Left Shift to capture selected text.
- Closing the window hides it from the taskbar. Use the tray menu to select **Open Book Save**. A second launch reveals the same window and does not start another copy.
- Book Save opens after capture. Review the selected text, then press **Enter** or select **Add** to save it.
- To stop capture, select **Exit** in the tray menu. Pending saves finish before Book Save closes.

## Saved list

Book Save saves your list here:

```text
%LOCALAPPDATA%\Book Save\captures.json
```

The app creates this file when you save your first item. If the app finds an older list in `book-save-app\data\captures.json`, it copies that file to the new location. It keeps the old file as a backup. It does not replace a list that already exists in the new location.

Back up `captures.json` if you need another copy. Book Save does not sync your list to another device.

## Remove Book Save

The uninstaller refuses to run while the app is still running. Select **Exit** in the tray menu. Wait for Book Save to close, then run the uninstaller. Open the folder that contains `uninstall.ps1` and run:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\uninstall.ps1
```

The uninstaller removes the app, its Start menu shortcut, and its sign-in startup shortcut. It keeps your saved list. To remove the list too, add `-DeleteSavedList`:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\uninstall.ps1 -DeleteSavedList
```
