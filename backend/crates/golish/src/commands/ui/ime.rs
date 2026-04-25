/// macOS-only IME (Input Method Editor) switching via Carbon Text Input Source Services.
/// Returns no-op results on non-macOS platforms.

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_void, CStr, CString};
    use std::os::raw::c_char;

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFArrayRef = *const c_void;
    type CFDictionaryRef = *const c_void;

    const UTF8: u32 = 0x0800_0100;

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        fn TISCopyCurrentKeyboardInputSource() -> CFTypeRef;
        fn TISSelectInputSource(source: CFTypeRef) -> i32;
        fn TISGetInputSourceProperty(source: CFTypeRef, key: CFStringRef) -> CFTypeRef;
        fn TISCreateInputSourceList(props: CFDictionaryRef, all: u8) -> CFArrayRef;
        static kTISPropertyInputSourceID: CFStringRef;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(alloc: CFTypeRef, s: *const c_char, enc: u32) -> CFStringRef;
        fn CFStringGetCStringPtr(s: CFStringRef, enc: u32) -> *const c_char;
        fn CFStringGetCString(s: CFStringRef, buf: *mut c_char, len: isize, enc: u32) -> u8;
        fn CFDictionaryCreate(
            alloc: CFTypeRef,
            keys: *const CFTypeRef,
            vals: *const CFTypeRef,
            count: isize,
            kcb: *const c_void,
            vcb: *const c_void,
        ) -> CFDictionaryRef;
        fn CFArrayGetCount(arr: CFArrayRef) -> isize;
        fn CFArrayGetValueAtIndex(arr: CFArrayRef, idx: isize) -> CFTypeRef;
        fn CFRelease(cf: CFTypeRef);
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
    }

    unsafe fn cfstr_to_string(cf: CFTypeRef) -> Option<String> {
        if cf.is_null() {
            return None;
        }
        let ptr = CFStringGetCStringPtr(cf, UTF8);
        if !ptr.is_null() {
            return CStr::from_ptr(ptr).to_str().ok().map(String::from);
        }
        let mut buf = [0u8; 256];
        if CFStringGetCString(cf, buf.as_mut_ptr().cast(), 256, UTF8) != 0 {
            CStr::from_ptr(buf.as_ptr().cast())
                .to_str()
                .ok()
                .map(String::from)
        } else {
            None
        }
    }

    pub fn get_current_input_source() -> Option<String> {
        unsafe {
            let source = TISCopyCurrentKeyboardInputSource();
            if source.is_null() {
                return None;
            }
            let prop = TISGetInputSourceProperty(source, kTISPropertyInputSourceID);
            let result = cfstr_to_string(prop);
            CFRelease(source);
            result
        }
    }

    pub fn select_input_source(source_id: &str) -> bool {
        unsafe {
            let cf_id = CFStringCreateWithCString(
                std::ptr::null(),
                CString::new(source_id).unwrap_or_default().as_ptr(),
                UTF8,
            );
            if cf_id.is_null() {
                return false;
            }

            let key: CFTypeRef = kTISPropertyInputSourceID;
            let dict = CFDictionaryCreate(
                std::ptr::null(),
                &key,
                &(cf_id as CFTypeRef),
                1,
                std::ptr::addr_of!(kCFTypeDictionaryKeyCallBacks).cast(),
                std::ptr::addr_of!(kCFTypeDictionaryValueCallBacks).cast(),
            );

            let sources = TISCreateInputSourceList(dict, 0);
            let ok = if !sources.is_null() && CFArrayGetCount(sources) > 0 {
                let src = CFArrayGetValueAtIndex(sources, 0);
                TISSelectInputSource(src) == 0
            } else {
                false
            };

            if !sources.is_null() {
                CFRelease(sources);
            }
            CFRelease(dict);
            CFRelease(cf_id);
            ok
        }
    }
}

#[tauri::command]
pub fn ime_get_source() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        macos::get_current_input_source()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[tauri::command]
pub fn ime_set_source(source_id: String) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::select_input_source(&source_id)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = source_id;
        false
    }
}
