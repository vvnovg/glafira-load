//! Имя приложения в Dock и переключателе ⌘Tab при `tauri dev`. Без бандла `.app`
//! LaunchServices показывает имя исполняемого файла (`glafira-load-testing`), а вшитый Info.plist
//! с `CFBundleName` не учитывает. Поэтому в debug-сборке имя задаётся вызовом LaunchServices —
//! так же переименовывает процессы Chromium. В релизе имя берётся из бандла, вызов не нужен.

/// Задаёт отображаемое имя процесса. `false` — LaunchServices не дал этого сделать.
#[cfg(all(target_os = "macos", debug_assertions))]
pub fn set(name: &str) -> bool {
    use std::ffi::{CStr, c_char, c_int, c_void};
    use std::ptr::{null, null_mut};

    type CFTypeRef = *const c_void;
    type GetAsn = unsafe extern "C" fn() -> CFTypeRef;
    type SetItem = unsafe extern "C" fn(c_int, CFTypeRef, CFTypeRef, CFTypeRef, *mut c_void) -> i32;

    const RTLD_LAZY: c_int = 0x1;
    const LS_DEFAULT_SESSION: c_int = -2;
    const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const LAUNCH_SERVICES: &CStr = c"/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/LaunchServices";

    unsafe extern "C" {
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithBytes(
            alloc: CFTypeRef,
            bytes: *const u8,
            len: isize,
            encoding: u32,
            external: u8,
        ) -> CFTypeRef;
        fn CFRelease(cf: CFTypeRef);
    }

    // Функции LaunchServices закрытые: ищутся через dlsym, чтобы их пропажа в новой macOS
    // не мешала запуску.
    unsafe {
        let ls = dlopen(LAUNCH_SERVICES.as_ptr(), RTLD_LAZY);
        if ls.is_null() {
            return false;
        }
        let get_asn = dlsym(ls, c"_LSGetCurrentApplicationASN".as_ptr());
        let set_item = dlsym(ls, c"_LSSetApplicationInformationItem".as_ptr());
        let key = dlsym(ls, c"_kLSDisplayNameKey".as_ptr()).cast::<CFTypeRef>();
        if get_asn.is_null() || set_item.is_null() || key.is_null() {
            return false;
        }
        let get_asn = std::mem::transmute::<*mut c_void, GetAsn>(get_asn);
        let set_item = std::mem::transmute::<*mut c_void, SetItem>(set_item);
        let asn = get_asn();
        if asn.is_null() {
            return false;
        }
        let value = CFStringCreateWithBytes(
            null(),
            name.as_ptr(),
            name.len() as isize,
            CF_STRING_ENCODING_UTF8,
            0,
        );
        if value.is_null() {
            return false;
        }
        let status = set_item(LS_DEFAULT_SESSION, asn, *key, value, null_mut());
        CFRelease(value);
        status == 0
    }
}

#[cfg(not(all(target_os = "macos", debug_assertions)))]
pub fn set(_name: &str) -> bool {
    true
}
