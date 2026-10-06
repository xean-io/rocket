//! Port of `adapters/events/bus_test.go`.

use rocket_adapters::events::Bus;
use rocket_domain::ports::EventBus;
use rocket_domain::{Event, event_type};
use time::macros::datetime;

fn log_event(line: &str) -> Event {
    Event {
        r#type: event_type::LOG_LINE.into(),
        time: datetime!(2026-01-01 00:00:00 UTC),
        project: String::new(),
        service: String::new(),
        state: None,
        line: line.into(),
        run: None,
        lease: None,
        job_id: String::new(),
        status: None,
        job: None,
    }
}

#[tokio::test]
async fn publish_subscribe() {
    let b = Bus::new();
    let mut sub = b.subscribe(1);
    b.publish(log_event("a"));
    b.publish(log_event("dropped")); // buffer full: must not block
    assert_eq!(sub.recv().await.unwrap().line, "a");
    assert!(sub.try_recv().is_err(), "the overflowing event is dropped");
    drop(sub);
    assert_eq!(b.subscriber_count(), 0);
    b.publish(log_event("")); // no subscribers: no panic
}

#[tokio::test]
async fn dropping_a_subscription_unsubscribes_only_that_subscriber() {
    let b = Bus::new();
    let mut keep = b.subscribe(8);
    let gone = b.subscribe(8);
    assert_eq!(b.subscriber_count(), 2);
    drop(gone);
    assert_eq!(b.subscriber_count(), 1);
    b.publish(log_event("x"));
    assert_eq!(keep.recv().await.unwrap().line, "x");
}

#[tokio::test]
async fn zero_buffer_defaults_to_256() {
    let b = Bus::new();
    let mut sub = b.subscribe(0);
    for i in 0..300 {
        b.publish(log_event(&i.to_string()));
    }
    let mut n = 0;
    while sub.try_recv().is_ok() {
        n += 1;
    }
    assert_eq!(n, 256);
}

#[tokio::test]
async fn slow_subscribers_do_not_starve_fast_ones() {
    let b = Bus::new();
    let _slow = b.subscribe(1);
    let mut fast = b.subscribe(16);
    for i in 0..5 {
        b.publish(log_event(&i.to_string()));
    }
    for i in 0..5 {
        assert_eq!(fast.recv().await.unwrap().line, i.to_string());
    }
}
