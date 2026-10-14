fn format_name(format: u32) -> String {
    let standard = match format {
        1 => "CF_TEXT",
        2 => "CF_BITMAP",
        3 => "CF_METAFILEPICT",
        7 => "CF_OEMTEXT",
        8 => "CF_DIB",
        9 => "CF_PALETTE",
        13 => "CF_UNICODETEXT",
        14 => "CF_ENHMETAFILE",
        15 => "CF_HDROP",
        16 => "CF_LOCALE",
        17 => "CF_DIBV5",
        _ => "",
    };
    if !standard.is_empty() {
        return standard.into();
    }
    let mut buffer = [0u16; 256];
    let length =
        unsafe { GetClipboardFormatNameW(format, buffer.as_mut_ptr(), buffer.len() as i32) };
    if length > 0 {
        String::from_utf16_lossy(&buffer[..length as usize])
    } else {
        "unknown".into()
    }
}

pub fn run_probe() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("source") {
        return source_window(args.get(2).ok_or("Missing source handle path")?);
    }
    if args.get(1).map(String::as_str) == Some("verify") {
        return verify_capture();
    }
    let owner = ClipboardWindow::new()?;
    let _clipboard = Clipboard::open(owner.0)?;
    let mut format = 0;
    loop {
        format = unsafe { EnumClipboardFormats(format) };
        if format == 0 {
            break;
        }
        println!("FORMAT {format} {}", format_name(format));
    }
    let backup = backup_clipboard()?;
    println!(
        "PASS backed up {} actual clipboard formats without changing the clipboard",
        backup.len()
    );
    Ok(())
}

const EXPECTED_SELECTION: &str = "Book Save selected text: café नमस्ते 😀";

fn source_window(handle_path: &str) -> Result<(), String> {
    let class: Vec<u16> = "EDIT\0".encode_utf16().collect();
    let text: Vec<u16> = EXPECTED_SELECTION.encode_utf16().chain([0]).collect();
    let window = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            text.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE | 0x0004 | 0x0100,
            80,
            80,
            540,
            160,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        )
    };
    if window.is_null() {
        return Err("Could not create the external selected-text source".into());
    }
    unsafe {
        SetForegroundWindow(window);
        SetFocus(window);
        SendMessageW(window, 0x00B1, 0, -1);
    }
    std::fs::write(handle_path, (window as usize).to_string())
        .map_err(|error| error.to_string())?;
    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

struct SourceProcess {
    process: std::process::Child,
    window: HWND,
    handle_path: std::path::PathBuf,
    previous_foreground: HWND,
}

impl SourceProcess {
    fn start() -> Result<Self, String> {
        use std::os::windows::process::CommandExt;
        let previous_foreground = unsafe { GetForegroundWindow() };
        let handle_path =
            std::env::temp_dir().join(format!("book-save-verify-{}.hwnd", std::process::id()));
        let _ = std::fs::remove_file(&handle_path);
        let process =
            std::process::Command::new(std::env::current_exe().map_err(|error| error.to_string())?)
                .arg("source")
                .arg(&handle_path)
                .creation_flags(0x08000000)
                .spawn()
                .map_err(|error| error.to_string())?;
        let mut source = Self {
            process,
            window: null_mut(),
            handle_path,
            previous_foreground,
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(value) = std::fs::read_to_string(&source.handle_path) {
                source.window = value.parse::<usize>().map_err(|error| error.to_string())? as HWND;
                break;
            }
            if Instant::now() >= deadline {
                return Err("External source did not start".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
        source.focus()?;
        Ok(source)
    }

    fn focus(&self) -> Result<(), String> {
        use windows_sys::Win32::System::Threading::AttachThreadInput;
        unsafe {
            let foreground_thread = GetWindowThreadProcessId(GetForegroundWindow(), null_mut());
            let own_thread = GetCurrentThreadId();
            let attached = foreground_thread != own_thread
                && AttachThreadInput(own_thread, foreground_thread, 1) != 0;
            SetForegroundWindow(self.window);
            if attached {
                AttachThreadInput(own_thread, foreground_thread, 0);
            }
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        while unsafe { GetForegroundWindow() } != self.window {
            if Instant::now() >= deadline {
                return Err("Could not put the verification source in the foreground".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
}

impl Drop for SourceProcess {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
        let _ = std::fs::remove_file(&self.handle_path);
        unsafe {
            SetForegroundWindow(self.previous_foreground);
        }
    }
}

struct RestoreOriginal {
    owner: HWND,
    data: Option<Vec<(u32, ClipboardData)>>,
}

impl RestoreOriginal {
    fn restore(&mut self) -> Result<(), String> {
        let _clipboard = Clipboard::open(self.owner)?;
        if unsafe { EmptyClipboard() } == 0 {
            return Err("Verification could not restore the original clipboard".into());
        }
        for (format, data) in self
            .data
            .take()
            .ok_or("Original clipboard already restored")?
        {
            data.put(format)?;
        }
        Ok(())
    }
}

impl Drop for RestoreOriginal {
    fn drop(&mut self) {
        if self.data.is_some() {
            if let Err(error) = self.restore() {
                eprintln!("FAIL original clipboard recovery: {error}");
            }
        }
    }
}

fn bitmap_snapshot(handle: HBITMAP) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Graphics::Gdi::{
        BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, GetDIBits,
    };
    let mut bitmap = BITMAP::default();
    if unsafe {
        GetObjectW(
            handle,
            size_of::<BITMAP>() as i32,
            (&mut bitmap as *mut BITMAP).cast(),
        )
    } == 0
    {
        return Err("Could not inspect bitmap pixels".into());
    }
    let mut info = BITMAPINFO::default();
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: bitmap.bmWidth,
        biHeight: -bitmap.bmHeight.abs(),
        biPlanes: 1,
        biBitCount: 32,
        ..Default::default()
    };
    let mut pixels =
        vec![0u8; bitmap.bmWidth as usize * bitmap.bmHeight.unsigned_abs() as usize * 4];
    let dc = unsafe { CreateCompatibleDC(null_mut()) };
    if dc.is_null() {
        return Err("Could not allocate bitmap inspection DC".into());
    }
    let lines = unsafe {
        GetDIBits(
            dc,
            handle,
            0,
            bitmap.bmHeight.unsigned_abs(),
            pixels.as_mut_ptr().cast(),
            &mut info,
            DIB_RGB_COLORS,
        )
    };
    unsafe {
        DeleteDC(dc);
    }
    if lines != bitmap.bmHeight.abs() {
        return Err("Could not read bitmap pixels".into());
    }
    let mut bytes = bitmap.bmWidth.to_le_bytes().to_vec();
    bytes.extend(bitmap.bmHeight.to_le_bytes());
    bytes.extend(pixels);
    Ok(bytes)
}

fn palette_snapshot(
    handle: windows_sys::Win32::Graphics::Gdi::HPALETTE,
) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Graphics::Gdi::{GetPaletteEntries, PALETTEENTRY};
    let count = unsafe { GetPaletteEntries(handle, 0, 0, null_mut()) };
    if count == 0 {
        return Err("Could not inspect palette entries".into());
    }
    let mut entries = vec![PALETTEENTRY::default(); count as usize];
    if unsafe { GetPaletteEntries(handle, 0, count, entries.as_mut_ptr()) } != count {
        return Err("Could not read palette entries".into());
    }
    Ok(entries
        .iter()
        .flat_map(|entry| [entry.peRed, entry.peGreen, entry.peBlue, entry.peFlags])
        .collect())
}

fn snapshot(owner: HWND) -> Result<std::collections::BTreeMap<u32, Vec<u8>>, String> {
    let _clipboard = Clipboard::open(owner)?;
    let mut result = std::collections::BTreeMap::new();
    let mut format = 0;
    loop {
        format = unsafe { EnumClipboardFormats(format) };
        if format == 0 {
            break;
        }
        let handle = unsafe { GetClipboardData(format) };
        let bytes = match format {
            2 | 0x82 => bitmap_snapshot(handle.cast())?,
            9 => palette_snapshot(handle.cast())?,
            _ => clipboard_bytes(format)?,
        };
        result.insert(format, bytes);
    }
    Ok(result)
}

fn expect_restored(
    owner: HWND,
    expected: &std::collections::BTreeMap<u32, Vec<u8>>,
) -> Result<(), String> {
    let actual = snapshot(owner)?;
    for (format, bytes) in expected {
        if actual.get(format) != Some(bytes) {
            return Err(format!(
                "Restored {} differs from its original bytes or pixels",
                format_name(*format)
            ));
        }
    }
    if actual.len() != expected.len() {
        return Err("Restored clipboard format set differs".into());
    }
    Ok(())
}

fn dib_fixture(version5: bool, indexed: bool) -> Vec<u8> {
    let header_size = if version5 { 124 } else { 40 };
    let bit_count = if indexed { 8u16 } else { 32 };
    let row_size = if indexed { 4usize } else { 8 };
    let mut bytes = vec![0u8; header_size + if indexed { 8 } else { 0 } + row_size * 2];
    bytes[0..4].copy_from_slice(&(header_size as u32).to_le_bytes());
    bytes[4..8].copy_from_slice(&2i32.to_le_bytes());
    bytes[8..12].copy_from_slice(&2i32.to_le_bytes());
    bytes[12..14].copy_from_slice(&1u16.to_le_bytes());
    bytes[14..16].copy_from_slice(&bit_count.to_le_bytes());
    bytes[20..24].copy_from_slice(&(row_size as u32 * 2).to_le_bytes());
    if version5 {
        bytes[56..60].copy_from_slice(&0x73524742u32.to_le_bytes());
        bytes[108..112].copy_from_slice(&4u32.to_le_bytes());
    }
    if indexed {
        bytes[32..36].copy_from_slice(&2u32.to_le_bytes());
        bytes[header_size..header_size + 8].copy_from_slice(&[0, 0, 0, 0, 0, 255, 0, 0]);
        bytes[header_size + 8..].copy_from_slice(&[0, 1, 0, 0, 1, 0, 0, 0]);
    } else {
        bytes[header_size..]
            .copy_from_slice(&[0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0]);
    }
    bytes
}

fn text_bytes(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn set_fixture(owner: HWND, name: &str) -> Result<(), String> {
    use windows_sys::Win32::Graphics::Gdi::{
        CreateBitmap, CreatePalette, LOGPALETTE, PALETTEENTRY,
    };
    let _clipboard = Clipboard::open(owner)?;
    if unsafe { EmptyClipboard() } == 0 {
        return Err("Could not seed verification clipboard".into());
    }
    match name {
        "bitmap" => {
            let pixels = [0u32, 0x00FF0000, 0x0000FF00, 0x000000FF];
            let bitmap = unsafe { CreateBitmap(2, 2, 1, 32, pixels.as_ptr().cast()) };
            if bitmap.is_null() {
                return Err("Could not seed bitmap".into());
            }
            ClipboardData::Bitmap(bitmap).put(2)?;
        }
        "DIB" | "DIBV5" | "indexed DIB and palette" => {
            let indexed = name == "indexed DIB and palette";
            let version5 = name == "DIBV5";
            ClipboardData::from_bytes(&dib_fixture(version5, indexed))?.put(if version5 {
                17
            } else {
                8
            })?;
            if indexed {
                #[repr(C)]
                struct Palette {
                    version: u16,
                    count: u16,
                    entries: [PALETTEENTRY; 2],
                }
                let palette = Palette {
                    version: 0x300,
                    count: 2,
                    entries: [
                        PALETTEENTRY::default(),
                        PALETTEENTRY {
                            peRed: 0,
                            peGreen: 255,
                            peBlue: 0,
                            peFlags: 0,
                        },
                    ],
                };
                let handle =
                    unsafe { CreatePalette((&palette as *const Palette).cast::<LOGPALETTE>()) };
                if handle.is_null() {
                    return Err("Could not seed palette".into());
                }
                if unsafe { SetClipboardData(9, handle.cast()) }.is_null() {
                    unsafe {
                        DeleteObject(handle.cast());
                    }
                    return Err("Could not publish palette".into());
                }
            }
        }
        "HTML and RTF" => {
            for (registered, data) in [
                (
                    "HTML Format",
                    b"<html><body>Original HTML</body></html>\0".as_slice(),
                ),
                (
                    "Rich Text Format",
                    b"{\\rtf1\\ansi Original rich text}\0".as_slice(),
                ),
            ] {
                let name: Vec<u16> = registered.encode_utf16().chain([0]).collect();
                let format = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
                ClipboardData::from_bytes(data)?.put(format)?;
            }
            ClipboardData::from_bytes(&text_bytes("Original rich-text clipboard"))?.put(13)?;
        }
        "file list" => {
            let mut bytes = vec![0u8; 20];
            bytes[0..4].copy_from_slice(&20u32.to_le_bytes());
            bytes[16..20].copy_from_slice(&1u32.to_le_bytes());
            bytes.extend(text_bytes("C:\\Book Save verification\\fixture.txt\0"));
            ClipboardData::from_bytes(&bytes)?.put(15)?;
        }
        "standard memory formats" => {
            for format in [4, 5, 6, 11, 12, 0x81] {
                ClipboardData::from_bytes(b"Original opaque format data\0")?.put(format)?;
            }
        }
        _ => ClipboardData::from_bytes(&text_bytes("Original plain-text clipboard"))?.put(13)?,
    }
    Ok(())
}

fn verify_unsupported_refusal(owner: HWND, source: &SourceProcess) -> Result<(), String> {
    set_fixture(owner, "plain text")?;
    let private_handle = {
        let _clipboard = Clipboard::open(owner)?;
        let data = ClipboardData::from_bytes(b"Private owner-managed fixture\0")?;
        let handle = match &data {
            ClipboardData::Global(handle) => *handle,
            _ => unreachable!(),
        };
        data.put(0x0200)?;
        handle
    };
    let result = (|| {
        let expected = snapshot(owner)?;
        let sequence = unsafe { GetClipboardSequenceNumber() };
        source.focus()?;
        let error = capture_selection(owner, source.window, &AtomicBool::new(false))
            .err()
            .ok_or("Unsupported private format unexpectedly captured")?;
        if !error.contains("(512)") {
            return Err(format!("Unexpected private-format error: {error}"));
        }
        expect_restored(owner, &expected)?;
        if unsafe { GetClipboardSequenceNumber() } != sequence {
            return Err("Refusal changed the clipboard sequence".into());
        }
        Ok(())
    })();
    {
        let _clipboard = Clipboard::open(owner)?;
        if unsafe { EmptyClipboard() } == 0 {
            return Err("Could not clear private verification fixture".into());
        }
        unsafe {
            GlobalFree(private_handle);
        }
    }
    result?;
    println!("PASS unsupported private format refused with identical clipboard data and sequence");
    Ok(())
}

fn verify_capture() -> Result<(), String> {
    use windows_sys::Win32::System::Threading::{GR_GDIOBJECTS, GetGuiResources};
    let owner = ClipboardWindow::new()?;
    let original = {
        let _clipboard = Clipboard::open(owner.0)?;
        backup_clipboard()?
    };
    let mut restore = RestoreOriginal {
        owner: owner.0,
        data: Some(original),
    };
    let original_snapshot = snapshot(owner.0)?;
    let source = SourceProcess::start()?;
    for fixture in [
        "actual user clipboard",
        "plain text",
        "HTML and RTF",
        "bitmap",
        "DIB",
        "DIBV5",
        "indexed DIB and palette",
        "file list",
        "standard memory formats",
    ] {
        if fixture != "actual user clipboard" {
            set_fixture(owner.0, fixture)?;
        }
        let expected = snapshot(owner.0)?;
        source.focus()?;
        let selected = capture_selection(owner.0, source.window, &AtomicBool::new(false))?;
        if selected != EXPECTED_SELECTION {
            return Err(format!("{fixture}: captured text differs: {selected:?}"));
        }
        expect_restored(owner.0, &expected)?;
        println!(
            "PASS {fixture}: exact selected Unicode text captured; all {} formats restored",
            expected.len()
        );
    }
    source.focus()?;
    let (events, mut receiver) = futures::channel::mpsc::unbounded();
    let mut hook = CaptureWorker::start(events);
    let deadline = Instant::now() + Duration::from_secs(1);
    let ready = loop {
        match receiver.try_recv() {
            Ok(super::CaptureEvent::Ready) => break true,
            Ok(_) => break false,
            Err(e) if e.is_closed() => break false,
            Err(_) => {
                if Instant::now() >= deadline {
                    break false;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    };
    if !ready {
        return Err("Global keyboard hook did not start".into());
    }
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_LSHIFT,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let injected = [key(0), key(KEYEVENTF_KEYUP), key(0), key(KEYEVENTF_KEYUP)];
    if unsafe {
        SendInput(
            injected.len() as u32,
            injected.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    } != injected.len() as u32
    {
        return Err("Could not exercise injected key rejection".into());
    }
    if receiver.try_recv().is_ok() {
        return Err("Injected double Shift unexpectedly triggered capture".into());
    }
    thread::sleep(Duration::from_millis(200));
    if receiver.try_recv().is_ok() {
        return Err("Injected double Shift unexpectedly triggered capture".into());
    }
    hook.stop();
    println!("PASS real global keyboard hook rejects injected double Shift");
    let before_no_selection = snapshot(owner.0)?;
    unsafe {
        SendMessageW(source.window, 0x00B1, 0, 0);
    }
    source.focus()?;
    let error = capture_selection(owner.0, source.window, &AtomicBool::new(false))
        .err()
        .ok_or("No selection unexpectedly captured text")?;
    if !error.contains("No selected text") {
        return Err(format!("Unexpected no-selection error: {error}"));
    }
    expect_restored(owner.0, &before_no_selection)?;
    unsafe {
        SendMessageW(source.window, 0x00B1, 0, -1);
    }
    println!("PASS no-selection timeout restores every original clipboard format");
    verify_unsupported_refusal(owner.0, &source)?;
    set_fixture(owner.0, "bitmap")?;
    let gdi_before = unsafe {
        GetGuiResources(
            windows_sys::Win32::System::Threading::GetCurrentProcess(),
            GR_GDIOBJECTS,
        )
    };
    for _ in 0..20 {
        source.focus()?;
        if capture_selection(owner.0, source.window, &AtomicBool::new(false))? != EXPECTED_SELECTION
        {
            return Err("Repeated capture returned incorrect text".into());
        }
    }
    let gdi_after = unsafe {
        GetGuiResources(
            windows_sys::Win32::System::Threading::GetCurrentProcess(),
            GR_GDIOBJECTS,
        )
    };
    if gdi_after > gdi_before {
        return Err(format!(
            "Repeated capture leaked GDI objects: {gdi_before} -> {gdi_after}"
        ));
    }
    println!("PASS 20 repeated image captures: GDI objects {gdi_before} -> {gdi_after}");
    restore.restore()?;
    expect_restored(owner.0, &original_snapshot)?;
    println!(
        "PASS original user clipboard restored with identical bytes, bitmap pixels, and format set"
    );
    Ok(())
}
