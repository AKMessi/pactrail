
#[test]
fn pactrail_hidden_reserve_capacity() {
    let mut headers = HeaderMap::<usize>::default();
    assert_eq!(headers.capacity(), 0);

    let requested = 8;
    headers.reserve(requested);
    let reserved = headers.capacity();
    assert!(
        reserved >= requested,
        "reserve({requested}) exposed capacity {reserved}"
    );

    for index in 0..requested {
        let name = format!("x-pactrail-{index}")
            .parse::<HeaderName>()
            .expect("valid generated header name");
        headers.insert(name, index);
    }

    assert_eq!(
        headers.capacity(),
        reserved,
        "inserting the reserved number of entries reallocated"
    );
}
