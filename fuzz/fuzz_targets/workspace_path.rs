#![no_main]

use libfuzzer_sys::fuzz_target;
use pactrail_workspace::SafeRelativePath;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(path) = SafeRelativePath::new(text) else {
        return;
    };
    assert!(!path.as_path().is_absolute());
    let portable = path.portable();
    assert!(!portable.is_empty());
    assert!(!portable.split('/').any(|component| component == ".."));
    assert!(SafeRelativePath::new(&portable).is_ok());
});
