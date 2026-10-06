//! Session locks against a live PostgreSQL, from two "nodes" sharing it.
//!
//!     DATABASE_URL=postgres://postgres:postgres@localhost:5432/postrust_test \
//!         cargo test -p postrust-ai --test session_pg -- --ignored

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{manager, store, stub_model, stub_server, unique};
use postrust_ai::manager::{AcquireOptions, ModelManager};
use postrust_ai::queue::Priority;
use postrust_ai::session::Sessions;
use sqlx::PgPool;
use uuid::Uuid;

/// A node: its own manager with one loaded instance of `model`, and its own
/// session storage on the shared database.
struct TestNode {
    sessions: Arc<Sessions>,
    manager: ModelManager,
    instance: Uuid,
}

async fn node(pool: &PgPool, model: &str, url: &str) -> TestNode {
    let id = unique("node");
    let manager = manager(&id, &[stub_model(model, url, 4)]).await;
    let lease = manager
        .acquire(
            model,
            AcquireOptions::new(Priority::Normal, Duration::from_secs(5)),
        )
        .await
        .unwrap();
    let instance = lease.instance_id();
    drop(lease);
    let sessions = Arc::new(Sessions::new(
        pool.clone(),
        id,
        Duration::from_secs(600),
        manager.clone(),
    ));
    TestNode {
        sessions,
        manager,
        instance,
    }
}

async fn setup() -> (PgPool, String, TestNode, TestNode) {
    let (pool, _) = store().await;
    let url = stub_server(Duration::ZERO).await;
    let model = unique("model");
    let a = node(&pool, &model, &url).await;
    let b = node(&pool, &model, &url).await;
    (pool, model, a, b)
}

async fn expire(pool: &PgPool, session: &str, model: &str) {
    sqlx::query(
        "UPDATE ai.sessions SET expires_at = now() - interval '1 second'
         WHERE session_id = $1 AND model_id = $2",
    )
    .bind(session)
    .bind(model)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
#[ignore]
async fn concurrent_binds_have_exactly_one_winner() {
    let (_pool, model, a, b) = setup().await;
    // Several rounds, each a fresh session, since one lucky interleaving
    // proves nothing.
    for round in 0..8 {
        let session = unique(&format!("race{round}"));
        let (la, lb) = tokio::join!(
            a.sessions.bind(&session, &model, a.instance, None),
            b.sessions.bind(&session, &model, b.instance, None),
        );
        let (la, lb) = (la.unwrap(), lb.unwrap());

        // Both callers are told the same owner, and it is one of them.
        assert_eq!(la, lb, "the loser gets the winner's lock");
        let a_won = la.node_id == a.sessions.node_id();
        let b_won = la.node_id == b.sessions.node_id();
        assert!(a_won ^ b_won, "exactly one winner: {la:?}");
        assert_eq!(
            a.sessions.lookup(&session, &model).await.unwrap(),
            Some(la.clone())
        );

        // Only the winner counted the session against its instance.
        let winner = if a_won { &a } else { &b };
        assert_eq!(la.instance_id, winner.instance);
        assert!(winner.manager.session_count(winner.instance).unwrap() >= 1);
    }
    // Across all rounds each node's count equals the locks it holds.
    for n in [&a, &b] {
        let held = n.sessions.list().await.unwrap().len();
        assert_eq!(n.manager.session_count(n.instance), Some(held));
    }
}

#[tokio::test]
#[ignore]
async fn a_live_lock_is_not_taken_over() {
    let (_pool, model, a, b) = setup().await;
    let session = unique("live");
    let held = a
        .sessions
        .bind(&session, &model, a.instance, None)
        .await
        .unwrap();
    let again = b
        .sessions
        .bind(&session, &model, b.instance, None)
        .await
        .unwrap();
    assert_eq!(again, held);
    assert_eq!(b.manager.session_count(b.instance), Some(0));
}

#[tokio::test]
#[ignore]
async fn an_expired_lock_is_taken_over() {
    let (pool, model, a, b) = setup().await;
    let session = unique("expired");
    a.sessions
        .bind(&session, &model, a.instance, None)
        .await
        .unwrap();
    expire(&pool, &session, &model).await;
    assert_eq!(
        a.sessions.lookup(&session, &model).await.unwrap(),
        None,
        "an expired lock is not live"
    );

    let taken = b
        .sessions
        .bind(&session, &model, b.instance, None)
        .await
        .unwrap();
    assert_eq!(taken.node_id, b.sessions.node_id());
    assert_eq!(taken.instance_id, b.instance);
    assert_eq!(b.manager.session_count(b.instance), Some(1));
    assert_eq!(
        a.sessions.lookup(&session, &model).await.unwrap(),
        Some(taken)
    );
}

#[tokio::test]
#[ignore]
async fn a_lock_on_a_dead_instance_is_replaced_even_before_it_expires() {
    let (_pool, model, a, b) = setup().await;
    let session = unique("replacing");
    a.sessions
        .bind(&session, &model, a.instance, None)
        .await
        .unwrap();

    // Without `replacing`, B loses; naming the old instance as gone wins.
    let lost = b
        .sessions
        .bind(&session, &model, b.instance, None)
        .await
        .unwrap();
    assert_eq!(lost.node_id, a.sessions.node_id());
    let won = b
        .sessions
        .bind(&session, &model, b.instance, Some(a.instance))
        .await
        .unwrap();
    assert_eq!(won.node_id, b.sessions.node_id());
    assert_eq!(won.instance_id, b.instance);
    assert_eq!(b.manager.session_count(b.instance), Some(1));

    // A is not told; its next maintenance drops the count for a session it
    // no longer owns.
    assert_eq!(a.manager.session_count(a.instance), Some(1));
    a.sessions.maintain().await.unwrap();
    assert_eq!(a.manager.session_count(a.instance), Some(0));
}

#[tokio::test]
#[ignore]
async fn maintain_resyncs_counts_after_a_release_on_the_other_node() {
    let (_pool, model, a, b) = setup().await;
    let first = unique("sync1");
    let second = unique("sync2");
    for s in [&first, &second] {
        a.sessions.bind(s, &model, a.instance, None).await.unwrap();
    }
    assert_eq!(a.manager.session_count(a.instance), Some(2));

    // B deletes the row; only the owner's count is stale until it maintains.
    assert!(b.sessions.release(&first, &model).await.unwrap());
    assert_eq!(a.manager.session_count(a.instance), Some(2));
    a.sessions.maintain().await.unwrap();
    assert_eq!(a.manager.session_count(a.instance), Some(1));

    // And the reverse: an expired lock is deleted by its owner's maintain.
    let (pool, _) = store().await;
    expire(&pool, &second, &model).await;
    a.sessions.maintain().await.unwrap();
    assert_eq!(a.manager.session_count(a.instance), Some(0));
    assert!(a.sessions.list().await.unwrap().is_empty());
}
