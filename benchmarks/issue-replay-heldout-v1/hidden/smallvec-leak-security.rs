use smallvec::SmallVec;
use std::panic::{catch_unwind, AssertUnwindSafe};

#[test]
fn inline_storage_leak_is_rejected() {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut values: SmallVec<u8, 4> = SmallVec::new();
        values.extend([1, 2, 3]);
        let _ = values.leak();
    }));

    assert!(result.is_err(), "leaking inline storage must panic");
}

#[test]
fn heap_storage_can_still_be_leaked() {
    let mut values: SmallVec<u8, 2> = SmallVec::with_capacity(8);
    values.extend([4, 5, 6]);
    let leaked = values.leak();
    assert_eq!(leaked, [4, 5, 6]);
}
