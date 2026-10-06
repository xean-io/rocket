//! Ports of `project_metadata_test.go`.

use crate::testing::*;

#[tokio::test]
async fn summary_declared_project_lists() {
    for empty in [false, true] {
        let name = if empty {
            "empty arrays"
        } else {
            "declared sorted"
        };
        let h = Harness::new();
        let mut p = jobs_project();
        let mut want: [(&str, &[&str]); 3] = [
            ("envs", &["dev", "prod", "stage"]),
            ("pipelines", &["broken", "ci", "slow"]),
            ("deploy_envs", &["prod", "stage"]),
        ];
        if empty {
            p.envs.clear();
            p.pipelines.clear();
            want = [("envs", &[]), ("pipelines", &[]), ("deploy_envs", &[])];
        }
        h.loader.set(p.clone());
        let summary = h.app.summary(&p.root).unwrap();
        let project = summary.project.expect("summary has a project");
        let wire = serde_json::to_value(&project).unwrap();
        for (key, expected) in want {
            let got = wire[key].as_array().unwrap_or_else(|| {
                panic!(
                    "{name}: project.{key} = {} is not a non-null array",
                    wire[key]
                )
            });
            let got: Vec<&str> = got.iter().map(|v| v.as_str().unwrap()).collect();
            assert_eq!(got, expected, "{name}: project.{key}");
        }
        assert!(
            project.name == p.name
                && project.root == p.root
                && project.default_env == p.default_env,
            "{name}: legacy project fields changed: {project:?}"
        );
    }
}
