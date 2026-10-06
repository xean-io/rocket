//! Go `url.Values` / `strconv` semantics used by the handlers.

use rocket_api::query::Query;

#[test]
fn first_value_wins_and_missing_is_empty() {
    let q = Query::parse(Some("a=1&a=2&b=&c"));
    assert_eq!(q.get("a"), "1");
    assert_eq!(q.get("b"), "");
    assert_eq!(q.get("c"), "");
    assert_eq!(q.get("zzz"), "");
    assert_eq!(Query::parse(None).get("a"), "");
}

#[test]
fn percent_and_plus_decoding() {
    let q = Query::parse(Some("p=%2Ftmp%2Fmy%20proj&q=a+b&%6Bey=v"));
    assert_eq!(q.get("p"), "/tmp/my proj");
    assert_eq!(q.get("q"), "a b");
    assert_eq!(q.get("key"), "v");
}

#[test]
fn malformed_pairs_are_dropped_like_go() {
    // Bad escape and semicolons: Go's ParseQuery skips the pair, keeps the rest.
    let q = Query::parse(Some("bad=%zz&semi=1;x=2&ok=yes&&"));
    assert_eq!(q.get("bad"), "");
    assert_eq!(q.get("semi"), "");
    assert_eq!(q.get("ok"), "yes");
}

#[test]
fn flag_matches_strconv_parse_bool() {
    for t in ["1", "t", "T", "TRUE", "true", "True"] {
        assert!(Query::parse(Some(&format!("f={t}"))).flag("f"), "{t}");
    }
    for f in [
        "0", "f", "F", "FALSE", "false", "False", "yes", "on", "TrUe", "",
    ] {
        assert!(!Query::parse(Some(&format!("f={f}"))).flag("f"), "{f}");
    }
    assert!(!Query::parse(None).flag("f"));
}

#[test]
fn int_matches_strconv_atoi() {
    let get = |v: &str| Query::parse(Some(&format!("n={v}"))).int("n");
    assert_eq!(get("5"), Some(5));
    assert_eq!(get("%2B5"), Some(5));
    // A bare '+' in a query is a space, which Atoi rejects.
    assert_eq!(get("+5"), None);
    assert_eq!(get("-3"), Some(-3));
    assert_eq!(get("0"), Some(0));
    assert_eq!(get("abc"), None);
    assert_eq!(get("5x"), None);
    assert_eq!(get(" 5"), None);
    assert_eq!(get(""), None);
    assert_eq!(Query::parse(None).int("n"), None);
}
