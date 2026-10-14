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

The installer copies the app to `%LOCALAPPDATA%\Programs\Book Save`. It adds **Book Save** to the Start menu. It also adds **Book Save Background** to your personal Startup folder, so global capture keeps working after sign-in. Open it from the Start menu to run the app. The app shows a **BS** monogram icon in the Start menu, taskbar, and Startup shortcuts.

If Book Save is already running, the installer refuses to continue instead of closing the app for you. Open Book Save from the Start menu (or its taskbar button) and select **Quit** in the app header so any pending save can finish, then run the installer again.

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
- Book Save starts automatically without a visible window each time you sign in, so double-tapping Left Shift captures selected text even when you have not opened the window.
- Closing the window minimizes it to the taskbar instead of quitting, so capture keeps working. Select the taskbar button, or open Book Save from the Start menu again, to bring the window back. A second launch never starts a second copy; it reveals the running window.
- To quit Book Save completely, select **Quit** in the app header. Quitting stops global capture until you open the app again or sign in again.

## Saved list

Book Save saves your list here:

```text
%LOCALAPPDATA%\Book Save\captures.json
```

The app creates this file when you save your first item. If the app finds an older list in `book-save-app\data\captures.json`, it copies that file to the new location. It keeps the old file as a backup. It does not replace a list that already exists in the new location.

Back up `captures.json` if you need another copy. Book Save does not sync your list to another device.

## Remove Book Save

The uninstaller refuses to run while the app is still running. Open Book Save and select **Quit** in the app header so any pending save can finish, then run the uninstaller again. Open the folder that contains `uninstall.ps1` and run:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\uninstall.ps1
```

The uninstaller removes the app, its Start menu shortcut, and its sign-in startup shortcut. It keeps your saved list. To remove the list too, add `-DeleteSavedList`:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\uninstall.ps1 -DeleteSavedList
```
