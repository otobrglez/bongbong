//! The macOS app (tools/macos/bundle.sh): `BongBong.app/Contents/MacOS/bongbong`
//! with the assets in `Contents/Resources/static`. Finder starts an app with
//! `/` as its working directory, so `enter_bundle` moves into `Resources`,
//! where every `static/...` path resolves as it does in a checkout, and
//! keeps saved maps under the user's data directory, since a signed bundle
//! must not be written to. Every map, stamp and level ships inside the
//! binary, so `static/` is all the bundle carries. A binary run from
//! anywhere else - `cargo run`, an extracted folder - stays where it was
//! started, and `dock_icon` gives it the app's icon in the Dock, which
//! macOS otherwise draws as a generic executable's.

use std::ffi::{c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether this run is the binary in an app bundle, which carries its own
/// icon (`Contents/Resources/AppIcon.icns`).
static BUNDLED: AtomicBool = AtomicBool::new(false);

/// tools/macos/AppIcon.png, the picture the bundle's icns is made from.
static ICON_PNG: &[u8] = include_bytes!("../../tools/macos/AppIcon.png");

/// Work out of the app bundle's `Resources` when this binary is the one in
/// a bundle; otherwise leave the working directory alone.
pub fn enter_bundle() {
    let Some(resources) = std::env::current_exe().ok().as_deref().and_then(bundle_resources) else {
        return;
    };
    if !resources.join("static").is_dir() {
        return;
    }
    if let Err(e) = std::env::set_current_dir(&resources) {
        eprintln!("bongbong: cannot enter the app bundle {}: {e}", resources.display());
        return;
    }
    if let Some(dir) = crate::levels::data_dir() {
        crate::map::set_maps_dir(dir.join("maps"));
    }
    BUNDLED.store(true, Ordering::Relaxed);
    eprintln!("[bundle] working in {}", resources.display());
}

type Id = *mut c_void;

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Id;
    /// Called through a pointer cast to each message's own signature; on
    /// arm64 a variadic call would pass the arguments wrongly.
    fn objc_msgSend();
}

/// Put the app's icon in the Dock for a run outside the bundle. Call on
/// the main thread once the window is open: GLFW makes the application
/// object then, and the icon it starts with is the one the Dock shows.
pub fn dock_icon() {
    if BUNDLED.load(Ordering::Relaxed) {
        return;
    }
    // SAFETY: the main thread, after GLFW has made `NSApp`; every message
    // is sent through a pointer of the signature its method has, to
    // classes AppKit always provides, and a `nil` result is checked before
    // it is used. The two objects made with `alloc` are released here; the
    // application keeps its own reference to the image.
    unsafe {
        let send0: unsafe extern "C" fn(Id, Id) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let send1: unsafe extern "C" fn(Id, Id, Id) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let bytes: unsafe extern "C" fn(Id, Id, *const c_void, usize) -> Id =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let class = |name: &std::ffi::CStr| objc_getClass(name.as_ptr());
        let sel = |name: &std::ffi::CStr| sel_registerName(name.as_ptr());

        let data = send0(class(c"NSData"), sel(c"alloc"));
        let data = bytes(data, sel(c"initWithBytes:length:"), ICON_PNG.as_ptr().cast(), ICON_PNG.len());
        if data.is_null() {
            return;
        }
        let image = send1(send0(class(c"NSImage"), sel(c"alloc")), sel(c"initWithData:"), data);
        send0(data, sel(c"release"));
        if image.is_null() {
            eprintln!("bongbong: the Dock icon did not decode");
            return;
        }
        let app = send0(class(c"NSApplication"), sel(c"sharedApplication"));
        if !app.is_null() {
            send1(app, sel(c"setApplicationIconImage:"), image);
        }
        send0(image, sel(c"release"));
    }
}

/// The `Resources` folder of the bundle `exe` sits in - `X.app/Contents/MacOS/exe`
/// gives `X.app/Contents/Resources` - judged by the path's names alone.
fn bundle_resources(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents").then(|| contents.join("Resources"))
}

#[cfg(test)]
mod tests {
    use super::bundle_resources;
    use std::path::{Path, PathBuf};

    #[test]
    fn a_binary_in_a_bundle_works_out_of_its_resources() {
        assert_eq!(
            bundle_resources(Path::new("/Applications/BongBong.app/Contents/MacOS/bongbong")),
            Some(PathBuf::from("/Applications/BongBong.app/Contents/Resources"))
        );
    }

    #[test]
    fn a_binary_anywhere_else_stays_where_it_was_started() {
        assert_eq!(bundle_resources(Path::new("/Users/me/bongbong/target/debug/bongbong")), None);
        assert_eq!(bundle_resources(Path::new("/tmp/MacOS/bongbong")), None);
        assert_eq!(bundle_resources(Path::new("bongbong")), None);
    }
}
