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
    let length = unsafe { GetClipboardFormatNameW(format, buffer.as_mut_ptr(), buffer.len() as i32) };
    if length > 0 {
        String::from_utf16_lossy(&buffer[..length as usize])
    } else {
        "unknown".into()
    }
}

pub fn run_probe() -> Result<(), String> {
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
    println!("PASS backed up {} actual clipboard formats without changing the clipboard", backup.len());
    Ok(())
}
