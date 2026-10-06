//! Ports of `compose_reconcile_test.go`.

use crate::testing::*;

#[tokio::test]
async fn reconcile_compose_uses_loaded_alias() {
    let h = Harness::new();
    let mut p = nuvara();
    p.services.get_mut("postgres").unwrap().compose = "database".into();
    h.loader.set(p.clone());
    h.up(nuvara_req(&["postgres"])).await;
    let res = h.app.reconcile().await.unwrap();
    assert!(
        res.dead.is_empty() && res.adopted.len() == 1,
        "live Compose alias marked dead: {res:?}"
    );
    let got = h.run(&p.name, "postgres");
    assert!(got.state.active(), "alias run no longer active: {got:?}");
}

#[tokio::test]
async fn reconcile_compose_missing_manifest_keeps_legacy_fallback() {
    let h = Harness::new();
    h.up(nuvara_req(&["postgres"])).await;
    h.loader.clear();
    let res = h.app.reconcile().await.unwrap();
    assert!(
        res.dead.is_empty() && res.adopted.len() == 1,
        "missing manifest lost the stored service fallback: {res:?}"
    );
}
