#![no_main]

use libfuzzer_sys::fuzz_target;
use pactrail_core::{EventEnvelope, EventHash, RunEvent, RunId, RunSnapshot};
use time::OffsetDateTime;

fuzz_target!(|data: &[u8]| {
    if let Ok(envelope) = serde_json::from_slice::<EventEnvelope>(data) {
        let _verified = envelope.verify();
        let mut snapshot = RunSnapshot::new(envelope.run_id);
        let _projected = snapshot.apply(&envelope);
    }

    let message = String::from_utf8_lossy(data).into_owned();
    let run_id = RunId::new();
    let envelope = EventEnvelope::new(
        run_id,
        0,
        OffsetDateTime::UNIX_EPOCH,
        EventHash::genesis(),
        RunEvent::NoteRecorded { message },
    )
    .unwrap_or_else(|error| unreachable!("note event must serialize: {error}"));
    assert!(envelope.verify().is_ok_and(|valid| valid));
    let encoded = serde_json::to_vec(&envelope)
        .unwrap_or_else(|error| unreachable!("event must serialize: {error}"));
    let decoded: EventEnvelope = serde_json::from_slice(&encoded)
        .unwrap_or_else(|error| unreachable!("event must deserialize: {error}"));
    assert_eq!(decoded, envelope);
});
