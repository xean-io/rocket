//! Request decoding with Go `DisallowUnknownFields` semantics. Every expected
//! message below was produced by Go's `encoding/json` (see the daemon golden
//! fixtures for the same texts through HTTP).

use rocket_api::decode::{ADD_PROJECT, DOWN, JOB, UP, decode};
use rocket_domain::api::{AddProjectRequest, DownRequest, JobRequest, UpRequest};

fn up(body: &str) -> Result<UpRequest, String> {
    decode(body.as_bytes(), &UP)
}

#[test]
fn decodes_a_full_request() {
    let req = up(r#"{"project":"p","services":["a","b"],"env":"dev","profiles":["x"],"owner":"agent:1","ttl":"5m"}"#).unwrap();
    assert_eq!(req.project, "p");
    assert_eq!(req.services, ["a", "b"]);
    assert_eq!(req.ttl, "5m");
}

#[test]
fn null_and_trailing_content_and_leading_whitespace() {
    assert_eq!(up("null").unwrap(), UpRequest::default());
    assert_eq!(up("\t{\"project\":\"ok\"}").unwrap().project, "ok");
    assert_eq!(up(r#"{"project":"p"} trailing"#).unwrap().project, "p");
    assert_eq!(up(r#"{"project":"x"}}"#).unwrap().project, "x");
}

#[test]
fn field_names_match_case_insensitively_like_go() {
    assert_eq!(
        up(r#"{"Project":"P","SERVICES":["s"]}"#).unwrap().services,
        ["s"]
    );
}

#[test]
fn null_fields_are_no_ops() {
    assert_eq!(
        up(r#"{"project":null,"services":null}"#).unwrap(),
        UpRequest::default()
    );
}

#[test]
fn duplicate_keys_last_wins() {
    assert_eq!(up(r#"{"project":"a","project":"b"}"#).unwrap().project, "b");
}

#[test]
fn eof_and_unexpected_eof() {
    assert_eq!(up(""), Err("EOF".into()));
    assert_eq!(up("  \n"), Err("EOF".into()));
    assert_eq!(up("{"), Err("unexpected EOF".into()));
    assert_eq!(up(r#"{"project":"#), Err("unexpected EOF".into()));
    assert_eq!(up(r#"{"project":"x""#), Err("unexpected EOF".into()));
}

#[test]
fn unknown_fields_are_rejected_in_document_order() {
    assert_eq!(
        up(r#"{"project":"x","bogus":1}"#),
        Err(r#"json: unknown field "bogus""#.into())
    );
    // The first problem in document order wins, whatever its kind.
    assert_eq!(
        up(r#"{"project":5,"bogus":1}"#),
        Err(
            "json: cannot unmarshal number into Go struct field UpRequest.project of type string"
                .into()
        )
    );
    assert_eq!(
        up(r#"{"zzz":1,"project":5}"#),
        Err(r#"json: unknown field "zzz""#.into())
    );
}

#[test]
fn syntax_errors_match_go() {
    let cases = [
        (
            "nope",
            "invalid character 'o' in literal null (expecting 'u')",
        ),
        (
            r#"{"project" "x"}"#,
            "invalid character '\"' after object key",
        ),
        (
            r#"{"a":1,}"#,
            "invalid character '}' looking for beginning of object key string",
        ),
        ("[1 2]", "invalid character '2' after array element"),
        (
            r#"{"project":tru}"#,
            "invalid character '}' in literal true (expecting 'e')",
        ),
        (
            r#"{"project":"a\qb"}"#,
            "invalid character 'q' in string escape code",
        ),
        (
            r#"{"project":-}"#,
            "invalid character '}' in numeric literal",
        ),
        (
            r#"{"project":1.}"#,
            "invalid character '}' after decimal point in numeric literal",
        ),
        (
            r#"{"project":1e}"#,
            "invalid character '}' in exponent of numeric literal",
        ),
        (
            r#"{"project":1e+}"#,
            "invalid character '}' in exponent of numeric literal",
        ),
        (
            r#"{"project":"\u12G4"}"#,
            "invalid character 'G' in \\u hexadecimal character escape",
        ),
        (
            "{\"project\":\"a\u{1}b\"}",
            "invalid character '\\x01' in string literal",
        ),
        (
            r#"{"project":nul}"#,
            "invalid character '}' in literal null (expecting 'l')",
        ),
        (
            r#"{"project":fals}"#,
            "invalid character '}' in literal false (expecting 'e')",
        ),
        (
            r#"{"project":t}"#,
            "invalid character '}' in literal true (expecting 'r')",
        ),
        (
            "\u{7f}",
            "invalid character '\\x7f' looking for beginning of value",
        ),
        ("é", "invalid character 'Ã' looking for beginning of value"),
        (
            "{'a':1}",
            "invalid character '\\'' looking for beginning of object key string",
        ),
        ("]", "invalid character ']' looking for beginning of value"),
        (
            "[,]",
            "invalid character ',' looking for beginning of value",
        ),
        (
            "{,}",
            "invalid character ',' looking for beginning of object key string",
        ),
        (
            r#"{"project":}"#,
            "invalid character '}' looking for beginning of value",
        ),
        (
            "\u{0}",
            "invalid character '\\x00' looking for beginning of value",
        ),
    ];
    for (input, want) in cases {
        assert_eq!(up(input), Err(want.to_string()), "input {input:?}");
    }
}

#[test]
fn top_level_value_that_is_not_an_object() {
    for (input, kind) in [
        ("[]", "array"),
        (r#""s""#, "string"),
        ("12", "number"),
        ("true", "bool"),
    ] {
        assert_eq!(
            up(input),
            Err(format!(
                "json: cannot unmarshal {kind} into Go value of type app.UpRequest"
            )),
            "{input}"
        );
    }
}

#[test]
fn field_type_mismatches_name_the_go_field_and_type() {
    assert_eq!(
        up(r#"{"project":5}"#),
        Err(
            "json: cannot unmarshal number into Go struct field UpRequest.project of type string"
                .into()
        )
    );
    assert_eq!(
        up(r#"{"services":"a"}"#),
        Err("json: cannot unmarshal string into Go struct field UpRequest.services of type []string".into())
    );
    assert_eq!(
        up(r#"{"services":[1]}"#),
        Err(
            "json: cannot unmarshal number into Go struct field UpRequest.services of type string"
                .into()
        )
    );
    assert_eq!(
        decode::<DownRequest>(br#"{"everywhere":"yes"}"#, &DOWN),
        Err("json: cannot unmarshal string into Go struct field DownRequest.everywhere of type bool".into())
    );
    assert_eq!(
        decode::<AddProjectRequest>(br#"{"path":{"a":1}}"#, &ADD_PROJECT),
        Err("json: cannot unmarshal object into Go struct field AddProjectRequest.path of type string".into())
    );
}

#[test]
fn job_request_schema() {
    let req: JobRequest = decode(
        br#"{"project":"p","kind":"deploy","name":"stage","yes":true,"args":["<&>"]}"#,
        &JOB,
    )
    .unwrap();
    assert_eq!(req.name, "stage");
    assert!(req.yes);
    assert_eq!(
        decode::<JobRequest>(br#"{"kind":7}"#, &JOB),
        Err("json: cannot unmarshal number into Go struct field JobRequest.kind of type domain.JobKind".into())
    );
}
