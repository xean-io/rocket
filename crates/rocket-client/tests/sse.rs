//! SSE parsing (WHATWG event-stream rules) and typed daemon events.

use rocket_client::{DaemonEvent, SseFrame, SseMessage, SseParser};
use rocket_domain::RunState;

fn feed_all(chunks: &[&[u8]]) -> Vec<SseFrame> {
    let mut p = SseParser::new();
    chunks.iter().flat_map(|c| p.feed(c)).collect()
}

fn msg(event: &str, data: &str) -> SseFrame {
    SseFrame::Message(SseMessage {
        event: event.into(),
        data: data.into(),
        id: None,
    })
}

#[test]
fn comments_and_messages_in_order() {
    let frames = feed_all(&[b": ok\n\n: ping\n\nevent: log.line\ndata: {\"a\":1}\n\n"]);
    assert_eq!(
        frames,
        vec![
            SseFrame::Comment("ok".into()),
            SseFrame::Comment("ping".into()),
            msg("log.line", "{\"a\":1}"),
        ]
    );
}

#[test]
fn recorded_fixture_events_sse() {
    let fixture = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/events.sse"
    ))
    .unwrap();
    let frames = SseParser::new().feed(&fixture);
    assert_eq!(frames.len(), 3, "{frames:?}");

    let SseFrame::Message(first) = &frames[0] else {
        panic!("{:?}", frames[0])
    };
    assert_eq!(first.event, "service.state");
    assert_eq!(frames[1], SseFrame::Comment("ping".into()));
    let SseFrame::Message(second) = &frames[2] else {
        panic!("{:?}", frames[2])
    };
    assert_eq!(second.event, "log.line");

    match DaemonEvent::from_message(first).unwrap() {
        DaemonEvent::ServiceState(e) => {
            assert_eq!(e.service, "api");
            assert_eq!(e.state, Some(RunState::Running));
            assert_eq!(e.run.unwrap().ports["http"], 3002);
        }
        other => panic!("{other:?}"),
    }
    match DaemonEvent::from_message(second).unwrap() {
        DaemonEvent::LogLine(e) => assert_eq!(e.line, "listening on :3002"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn byte_at_a_time_equals_whole_input() {
    let input =
        b": ok\r\n\r\nevent: a\r\ndata: x\r\ndata: y\r\n\r\n: ping\n\nevent: b\ndata: z\n\n";
    let whole = feed_all(&[input]);
    let mut p = SseParser::new();
    let mut split = Vec::new();
    for b in input {
        split.extend(p.feed(std::slice::from_ref(b)));
    }
    assert_eq!(whole, split);
    assert_eq!(whole.len(), 4);
}

#[test]
fn multi_line_data_joins_with_newline() {
    let frames = feed_all(&[b"event: x\ndata: one\ndata: two\ndata:\ndata: four\n\n"]);
    assert_eq!(frames, vec![msg("x", "one\ntwo\n\nfour")]);
}

#[test]
fn crlf_lone_cr_and_split_crlf() {
    let frames = feed_all(&[b"event: a\r\ndata: 1\r\n\r", b"\nevent: b\rdata: 2\r\r"]);
    assert_eq!(frames, vec![msg("a", "1"), msg("b", "2")]);
}

#[test]
fn default_event_name_is_message_and_name_resets() {
    let frames = feed_all(&[b"data: a\n\nevent: x\ndata: b\n\ndata: c\n\n"]);
    assert_eq!(
        frames,
        vec![msg("message", "a"), msg("x", "b"), msg("message", "c")]
    );
}

#[test]
fn only_one_leading_space_is_stripped_and_colonless_fields_are_empty() {
    let frames = feed_all(&[b"data:  two spaces\n\ndata:nospace\n\ndata\n\n"]);
    assert_eq!(
        frames,
        vec![
            msg("message", " two spaces"),
            msg("message", "nospace"),
            msg("message", "")
        ]
    );
}

#[test]
fn blank_lines_without_data_dispatch_nothing_and_unknown_fields_are_ignored() {
    let frames = feed_all(&[b"\n\nevent: only-name\n\nbogus: 1\n\n"]);
    assert!(frames.is_empty(), "{frames:?}");
}

#[test]
fn id_and_retry_fields() {
    let frames = feed_all(&[b"id: 7\nretry: 1500\nretry: soon\nevent: e\ndata: d\n\n"]);
    assert_eq!(
        frames,
        vec![
            SseFrame::Retry(1500),
            SseFrame::Message(SseMessage {
                event: "e".into(),
                data: "d".into(),
                id: Some("7".into())
            }),
        ]
    );
}

#[test]
fn multibyte_utf8_split_across_chunks() {
    let s = "data: caf\u{e9} \u{1F680}\n\n".as_bytes();
    for cut in 1..s.len() {
        let frames = feed_all(&[&s[..cut], &s[cut..]]);
        assert_eq!(
            frames,
            vec![msg("message", "caf\u{e9} \u{1F680}")],
            "cut at {cut}"
        );
    }
}

#[test]
fn typed_events_by_name() {
    let mk = |name: &str, json: &str| SseMessage {
        event: name.into(),
        data: json.into(),
        id: None,
    };
    let base = r#""time":"2026-10-05T00:14:11Z","project":"p""#;
    let cases = [
        (
            "port.leased",
            format!(
                r#"{{"type":"port.leased",{base},"service":"s","lease":{{"port":3000,"project":"p","service":"s","port_name":"http","created_at":"2026-10-05T00:14:10Z"}}}}"#
            ),
        ),
        (
            "port.released",
            format!(r#"{{"type":"port.released",{base},"service":"s"}}"#),
        ),
        (
            "job.log",
            format!(r#"{{"type":"job.log",{base},"job_id":"j1","line":"hi"}}"#),
        ),
        (
            "job.state",
            format!(r#"{{"type":"job.state",{base},"job_id":"j1","status":"succeeded"}}"#),
        ),
    ];
    let kinds: Vec<_> = cases
        .iter()
        .map(|(n, j)| DaemonEvent::from_message(&mk(n, j)).unwrap())
        .collect();
    assert!(matches!(kinds[0], DaemonEvent::PortLeased(_)));
    assert!(matches!(kinds[1], DaemonEvent::PortReleased(_)));
    assert!(matches!(kinds[2], DaemonEvent::JobLog(_)));
    assert!(matches!(kinds[3], DaemonEvent::JobState(_)));
    assert!(kinds[3].is_terminal_job_state());
    assert!(!kinds[2].is_terminal_job_state());
    assert_eq!(kinds[2].event().unwrap().job_id, "j1");
}

#[test]
fn unknown_event_is_raw_and_known_with_bad_json_is_error() {
    let m = SseMessage {
        event: "future.thing".into(),
        data: "not json".into(),
        id: None,
    };
    assert_eq!(
        DaemonEvent::from_message(&m).unwrap(),
        DaemonEvent::Raw(m.clone())
    );
    let bad = SseMessage {
        event: "log.line".into(),
        data: "{".into(),
        id: None,
    };
    assert!(DaemonEvent::from_message(&bad).is_err());
}
