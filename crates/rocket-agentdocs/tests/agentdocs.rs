//! Ports of Go's `internal/agentdocs` tests plus golden comparisons against
//! content captured from the Go implementation
//! (`rocket agent install --print --json`).

use rocket_agentdocs::{
    BEGIN_MARKER, END_MARKER, Options, TARGET_AGENTS, TARGET_BOTH, TARGET_CLAUDE, block, install,
    plan, skill, upsert_block,
};
use std::path::Path;

const GOLDEN_SKILL: &str = include_str!("fixtures/SKILL.md");
const GOLDEN_BLOCK: &str = include_str!("fixtures/block.md");

#[test]
fn skill_and_block_are_byte_identical_to_go() {
    assert_eq!(skill(), GOLDEN_SKILL);
    assert_eq!(block(), GOLDEN_BLOCK);
}

#[test]
fn upsert_block_is_idempotent() {
    let cases: [(&str, &str, bool); 4] = [
        ("empty file", "", true),
        (
            "appends to existing content",
            "# Agents\n\nBe nice.\n",
            true,
        ),
        (
            "replaces an outdated block",
            "# A\n<!-- rocket:begin -->\nold\n<!-- rocket:end -->\ntail\n",
            true,
        ),
        ("unchanged when current", "", false),
    ];
    for (name, existing, changed) in cases {
        let existing = if name == "unchanged when current" {
            format!("# A\n\n{}", block())
        } else {
            existing.to_owned()
        };
        let (got, did_change) = upsert_block(&existing);
        assert_eq!(did_change, changed, "{name}\n{got}");
        assert_eq!(got.matches(BEGIN_MARKER).count(), 1, "{name}: {got}");
        assert_eq!(got.matches(END_MARKER).count(), 1, "{name}: {got}");
        let (again, again_changed) = upsert_block(&got);
        assert!(
            !again_changed && again == got,
            "{name}: second upsert changed"
        );
        if existing.contains("tail\n") {
            assert!(
                got.ends_with("tail\n"),
                "{name}: content after block lost\n{got}"
            );
        }
        if existing.contains("Be nice.") {
            assert!(got.starts_with("# Agents\n\nBe nice.\n"), "{name}: {got}");
        }
    }
}

#[test]
fn upsert_block_exact_shapes() {
    assert_eq!(upsert_block(""), (block(), true));
    // A file without a trailing newline gets one, then a blank line, then the block.
    assert_eq!(upsert_block("x"), (format!("x\n\n{}", block()), true));
    // An end marker before the begin marker is not a managed block: append.
    let weird = format!("{END_MARKER}\n{BEGIN_MARKER}\n");
    let (got, changed) = upsert_block(&weird);
    assert!(changed);
    assert!(got.starts_with(&weird) && got.ends_with(&block()), "{got}");
}

#[test]
fn content_carries_the_rules() {
    for (name, text) in [("skill", skill()), ("block", block())] {
        for want in [
            "ROCKET_OWNER=agent:",
            "--ttl",
            "rocket down --owner",
            "--json",
            "nohup",
            "docker compose up",
            "task dev",
            "rocket run",
        ] {
            assert!(text.contains(want), "{name} lacks {want:?}");
        }
    }
    assert!(skill().starts_with("---\nname: rocket\ndescription: "));
}

fn resolve(home: &Path, project: &Path, p: &str) -> std::path::PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => project.join(p),
    }
}

#[test]
fn install_targets() {
    struct Case {
        name: &'static str,
        target: &'static str,
        global: bool,
        writes: &'static [&'static str],
        skips: &'static [&'static str],
    }
    let cases = [
        Case {
            name: "both, project skill",
            target: TARGET_BOTH,
            global: false,
            writes: &[".claude/skills/rocket/SKILL.md", "AGENTS.md", "CLAUDE.md"],
            skips: &["~/.claude/skills/rocket/SKILL.md"],
        },
        Case {
            name: "claude, global skill",
            target: TARGET_CLAUDE,
            global: true,
            writes: &["~/.claude/skills/rocket/SKILL.md", "CLAUDE.md"],
            skips: &["AGENTS.md", ".claude/skills/rocket/SKILL.md"],
        },
        Case {
            name: "agents only",
            target: TARGET_AGENTS,
            global: false,
            writes: &["AGENTS.md"],
            skips: &["CLAUDE.md", ".claude/skills/rocket/SKILL.md"],
        },
    ];
    for c in cases {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let opts = Options {
            target: c.target.into(),
            global: c.global,
            home: home.path().into(),
            project_root: project.path().into(),
        };
        let actions = install(&opts).unwrap();
        assert_eq!(actions.len(), c.writes.len(), "{}: {actions:?}", c.name);
        for w in c.writes {
            let data = std::fs::read_to_string(resolve(home.path(), project.path(), w))
                .unwrap_or_else(|e| panic!("{}: {w} not written: {e}", c.name));
            assert!(
                data.contains("ROCKET_OWNER=agent:"),
                "{}: {w}\n{data}",
                c.name
            );
        }
        for s in c.skips {
            assert!(
                !resolve(home.path(), project.path(), s).exists(),
                "{}: {s} must not be written",
                c.name
            );
        }
        for a in install(&opts).unwrap() {
            assert_eq!(
                a.action, "unchanged",
                "{}: second install not idempotent",
                c.name
            );
        }
    }
    let bad = Options {
        target: "vim".into(),
        home: tempfile::tempdir().unwrap().path().into(),
        project_root: tempfile::tempdir().unwrap().path().into(),
        ..Options::default()
    };
    assert!(install(&bad).is_err(), "unknown target accepted");
}

#[test]
fn install_reports_created_updated_unchanged() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let opts = Options {
        target: TARGET_AGENTS.into(),
        home: home.path().into(),
        project_root: project.path().into(),
        ..Options::default()
    };
    let agents = project.path().join("AGENTS.md");
    assert_eq!(install(&opts).unwrap()[0].action, "created");
    assert_eq!(install(&opts).unwrap()[0].action, "unchanged");
    std::fs::write(&agents, "# mine\n").unwrap();
    assert_eq!(install(&opts).unwrap()[0].action, "updated");
    let data = std::fs::read_to_string(&agents).unwrap();
    assert!(
        data.starts_with("# mine\n\n<!-- rocket:begin -->"),
        "{data}"
    );
}

#[test]
fn plan_lists_files_without_writing() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let opts = Options {
        target: TARGET_BOTH.into(),
        home: home.path().into(),
        project_root: project.path().into(),
        ..Options::default()
    };
    let planned = plan(&opts).unwrap();
    let kinds: Vec<_> = planned
        .iter()
        .map(|a| (a.kind.as_str(), a.action.as_str()))
        .collect();
    assert_eq!(
        kinds,
        [
            ("skill", "planned"),
            ("block", "planned"),
            ("block", "planned")
        ]
    );
    assert!(planned[0].path.ends_with(".claude/skills/rocket/SKILL.md"));
    assert!(planned[1].path.ends_with("AGENTS.md") && planned[2].path.ends_with("CLAUDE.md"));
    assert!(!project.path().join("AGENTS.md").exists());

    // Outside a project and without --global there is no skill directory;
    // with --global and no project root the blocks are skipped.
    let no_root = Options {
        target: TARGET_BOTH.into(),
        global: true,
        home: home.path().into(),
        ..Options::default()
    };
    assert_eq!(plan(&no_root).unwrap().len(), 1);
    let nothing = Options {
        target: TARGET_CLAUDE.into(),
        ..Options::default()
    };
    assert!(plan(&nothing).is_err());
}

#[test]
fn action_json_shape() {
    let home = tempfile::tempdir().unwrap();
    let opts = Options {
        target: TARGET_CLAUDE.into(),
        global: true,
        home: home.path().into(),
        ..Options::default()
    };
    let actions = install(&opts).unwrap();
    let json = serde_json::to_string(&actions[0]).unwrap();
    assert!(
        json.starts_with("{\"path\":\"")
            && json.ends_with(",\"kind\":\"skill\",\"action\":\"created\"}"),
        "{json}"
    );
}
