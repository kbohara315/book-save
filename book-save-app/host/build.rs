//! Windows-only executable resources (app icon) for the resident host.
//!
//! The `BS` monogram icon is shared with the UI executable and linked through
//! the Windows SDK resource compiler. Non-Windows builds skip this step.

fn main() {
    #[cfg(windows)]
    let _ = embed_resource::compile("book-save.rc", embed_resource::NONE);
}
