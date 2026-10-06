//! Byte-for-byte comparison with what the Go daemon returned for the same
//! requests. `golden/*.golden` were captured once from a Go rocketd by
//! `golden/capture.py` (scenario in `golden/scenario.tsv`); this replays the
//! scenario against the Rust daemon and compares after normalizing the values
//! that legitimately differ between runs (pids, times, ids, paths, ports,
//! build version).
#![cfg(unix)]

mod common;

use bytes::Bytes;
use common::{copy_fixture, paths_in, short_tmp, start};
use http_body_util::{BodyExt, Empty, Full};
use hyper::Request;
use hyper_util::rt::TokioIo;
use rocket_client::DaemonInfo;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

struct Step {
    name: String,
    transport: String,
    method: String,
    path: String,
    body: String,
    save: String,
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn scenario() -> Vec<Step> {
    let text = std::fs::read_to_string(golden_dir().join("scenario.tsv")).unwrap();
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            assert_eq!(f.len(), 6, "bad scenario line {l:?}");
            Step {
                name: f[0].into(),
                transport: f[1].into(),
                method: f[2].into(),
                path: f[3].into(),
                body: f[4].into(),
                save: f[5].into(),
            }
        })
        .collect()
}

async fn send<T>(io: T, method: &str, path: &str, body: Option<&str>) -> (u16, Bytes)
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(io))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "rocketd");
    let resp = match body {
        Some(b) => sender
            .send_request(
                builder
                    .body(http_body_util::Either::Left(Full::new(Bytes::from(
                        b.to_owned(),
                    ))))
                    .unwrap(),
            )
            .await
            .unwrap(),
        None => sender
            .send_request(
                builder
                    .body(http_body_util::Either::Right(Empty::<Bytes>::new()))
                    .unwrap(),
            )
            .await
            .unwrap(),
    };
    let status = resp.status().as_u16();
    (status, resp.into_body().collect().await.unwrap().to_bytes())
}

// --- normalization ----------------------------------------------------------

/// Replaces RFC3339 timestamps (`2026-10-06T18:49:45.410201Z`) with `<TS>`.
fn scrub_timestamps(s: &str) -> String {
    let b = s.as_bytes();
    let digit = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    let at = |i: usize, c: u8| b.get(i) == Some(&c);
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        let is_ts = (0..4).all(|k| digit(i + k))
            && at(i + 4, b'-')
            && (5..7).all(|k| digit(i + k))
            && at(i + 7, b'-')
            && (8..10).all(|k| digit(i + k))
            && at(i + 10, b'T')
            && (11..13).all(|k| digit(i + k))
            && at(i + 13, b':')
            && (14..16).all(|k| digit(i + k))
            && at(i + 16, b':')
            && (17..19).all(|k| digit(i + k));
        if is_ts {
            let mut j = i + 19;
            if at(j, b'.') {
                j += 1;
                while digit(j) {
                    j += 1;
                }
            }
            if at(j, b'Z') {
                out.push_str("<TS>");
                i = j + 1;
                continue;
            }
        }
        // `i` is on a char boundary: copy one char.
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Replaces job ids (`j` + 10 hex digits, delimited) with `<JOB>`.
fn scrub_job_ids(s: &str) -> String {
    let b = s.as_bytes();
    let alnum = |i: usize| b.get(i).is_some_and(u8::is_ascii_alphanumeric);
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'j'
            && (i == 0 || !alnum(i - 1))
            && (1..=10).all(|k| b.get(i + k).is_some_and(u8::is_ascii_hexdigit))
            && !alnum(i + 11)
        {
            out.push_str("<JOB>");
            i += 11;
            continue;
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Blanks the value of `"key": <number>` / `"key":<number>`.
fn scrub_number(s: &str, key: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    let needle = format!("\"{key}\":");
    while let Some(pos) = rest.find(&needle) {
        let after = pos + needle.len();
        out.push_str(&rest[..after]);
        let tail = &rest[after..];
        let ws = tail.len() - tail.trim_start_matches(' ').len();
        out.push_str(&tail[..ws]);
        let tail = &tail[ws..];
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        out.push('0');
        rest = &tail[digits..];
    }
    out.push_str(rest);
    out
}

/// Blanks the value of `"key": "<string>"`.
fn scrub_string(s: &str, key: &str, with: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    let needle = format!("\"{key}\":");
    while let Some(pos) = rest.find(&needle) {
        let after = pos + needle.len();
        out.push_str(&rest[..after]);
        let tail = &rest[after..];
        let ws = tail.len() - tail.trim_start_matches(' ').len();
        out.push_str(&tail[..ws]);
        let tail = &tail[ws..];
        if let Some(body) = tail.strip_prefix('"') {
            let end = body.find('"').unwrap_or(body.len());
            out.push('"');
            out.push_str(with);
            rest = &body[end..];
        } else {
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

fn normalize(raw: &str, home: &str, proj: &str) -> String {
    let mut s = raw.replace(home, "<HOME>").replace(proj, "<PROJ>");
    s = scrub_timestamps(&s);
    s = scrub_job_ids(&s);
    for key in ["pid", "pgid", "duration_ms"] {
        s = scrub_number(&s, key);
    }
    s = scrub_string(&s, "version", "<VERSION>");
    s = scrub_string(&s, "http", "<HTTP>");
    s
}

/// The fixture's ports are probed by `status`; skip rather than flake.
fn fixture_ports_busy() -> bool {
    [18431, 18432, 18433]
        .iter()
        .any(|p| std::net::TcpListener::bind(("127.0.0.1", *p)).is_err())
}

#[test]
fn normalizer_blanks_run_specific_values() {
    let raw = "{\n  \"id\": \"j24e8e8e32a\",\n  \"pid\": 56695,\n  \"started_at\": \"2026-10-06T18:49:45.410201Z\",\n  \"time\":\"2026-10-06T18:49:45Z\",\n  \"version\": \"0.1.2-0.2026\",\n  \"http\": \"http://127.0.0.1:61607\",\n  \"log_path\": \"/h/logs/jobs/j24e8e8e32a.log\",\n  \"x\":\"jason\"\n}";
    assert_eq!(
        normalize(raw, "/h", "/p"),
        "{\n  \"id\": \"<JOB>\",\n  \"pid\": 0,\n  \"started_at\": \"<TS>\",\n  \"time\":\"<TS>\",\n  \"version\": \"<VERSION>\",\n  \"http\": \"<HTTP>\",\n  \"log_path\": \"<HOME>/logs/jobs/<JOB>.log\",\n  \"x\":\"jason\"\n}"
    );
}

#[tokio::test]
async fn rust_responses_are_byte_identical_to_gos() {
    if fixture_ports_busy() {
        eprintln!(
            "skipping: a fixture port (18431-18433) is in use, `status` would report conflicts"
        );
        return;
    }
    let tmp = short_tmp();
    let paths = paths_in(&tmp);
    let proj = copy_fixture(&tmp.path().join("proj"));
    let d = start(&paths).await;
    let info = DaemonInfo::load(&paths.daemon_json).unwrap();
    let home = paths.home.to_str().unwrap().to_string();
    let proj_s = proj.to_str().unwrap().to_string();

    let mut saved: HashMap<String, String> = HashMap::from([("PROJ".to_string(), proj_s.clone())]);
    let mut failures = Vec::new();
    for step in scenario() {
        let mut path = step.path.clone();
        let mut body = step.body.clone();
        for (k, v) in &saved {
            path = path.replace(&format!("{{{k}}}"), v);
            body = body.replace(&format!("{{{k}}}"), v);
        }
        let body = (body != "-").then_some(body);
        let (status, bytes) = match step.transport.as_str() {
            "unix" => {
                let io = tokio::net::UnixStream::connect(&paths.socket)
                    .await
                    .unwrap();
                send(io, &step.method, &path, body.as_deref()).await
            }
            "tcp-notoken" => {
                let addr = info.http.trim_start_matches("http://");
                let io = tokio::net::TcpStream::connect(addr).await.unwrap();
                send(io, &step.method, &path, body.as_deref()).await
            }
            other => panic!("unknown transport {other}"),
        };
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        if !step.save.is_empty() && step.save != "-" {
            let (key, field) = step.save.split_once('=').unwrap();
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            saved.insert(key.into(), v[field].as_str().unwrap().into());
        }
        let golden = std::fs::read(golden_dir().join(format!("{}.golden", step.name))).unwrap();
        let golden = String::from_utf8(golden).unwrap();
        // The previous run's job ids and paths were replaced by placeholders
        // on both sides, so the golden needs the same normalization; its
        // capture-time home/proj are unknown, so derive them from the file.
        let (g_status, g_body) = golden.split_once('\n').unwrap();
        let g_home = guess_prefix(g_body, "/home");
        let g_proj = guess_prefix(g_body, "/proj");
        let want = format!("{g_status}\n{}", normalize(g_body, &g_home, &g_proj));
        let got = format!("{status}\n{}", normalize(&text, &home, &proj_s));
        if want != got {
            failures.push(format!("--- {} ---\nGo:\n{want}\nRust:\n{got}", step.name));
        }
    }
    d.stop().await.unwrap();
    assert!(
        failures.is_empty(),
        "{} responses differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The `/tmp/rkgXXXX/<leaf>` prefix used when the golden was captured
/// (capture.py puts the home and the project copy side by side).
fn guess_prefix(body: &str, leaf: &str) -> String {
    let Some(end) = body.find(leaf) else {
        return "<<none>>".into();
    };
    let start = body[..end].rfind(['"', ' ']).map_or(0, |i| i + 1);
    format!("{}{leaf}", &body[start..end])
}
