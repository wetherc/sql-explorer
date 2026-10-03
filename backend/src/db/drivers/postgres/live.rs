//! The tests of the PostgreSQL driver against a live server. The variable
//! `SQLX_LIVE_PG` names the server, see [`crate::db::drivers::live`].

use crate::db::drivers::live::{self, Server};
use crate::db::drivers::DatabaseDriver;
use crate::db::{
    ExecOptions, ObjectType, RelationType, Table, Trigger, TriggerEvent, TriggerTiming,
};
use crate::storage::DbType;
use std::time::{Duration, Instant};

const FIXTURE: &str = include_str!("../../../../live/fixtures/postgres.sql");

/// A database of one test, with the session that made it.
struct Scratch {
    admin: Box<dyn DatabaseDriver>,
    name: String,
}

impl Scratch {
    /// Makes a database with the fixture and opens a driver on it. A server
    /// that is not configured gives `None`.
    async fn open(tag: &str) -> Option<(Scratch, Box<dyn DatabaseDriver>, Server)> {
        let server = live::server("SQLX_LIVE_PG")?;
        let mut admin = server.open(DbType::Postgres, Some("postgres")).await;
        let name = live::unique_name(tag);
        live::run(admin.as_mut(), &format!("CREATE DATABASE {name}")).await;
        let mut driver = server.open(DbType::Postgres, Some(&name)).await;
        let scratch = Scratch { admin, name };
        if let Err(error) = live::load_fixture(driver.as_mut(), FIXTURE).await {
            scratch.remove().await;
            panic!("the fixture failed: {error}");
        }
        Some((scratch, driver, server))
    }

    /// Removes the database, and ends each session that still uses it.
    async fn remove(mut self) {
        let sql = format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name);
        live::run(self.admin.as_mut(), &sql).await;
    }
}

fn trigger(name: &str, timing: TriggerTiming, events: &[TriggerEvent], enabled: bool) -> Trigger {
    Trigger {
        name: name.to_string(),
        timing,
        events: events.to_vec(),
        enabled,
        replica: false,
    }
}

fn type_names(response: &crate::db::QueryResponse) -> Vec<String> {
    response.results[0]
        .columns
        .iter()
        .map(|column| column.type_name.clone())
        .collect()
}

/// Reads the CREATE text of a trigger of the schema `app`.
async fn trigger_text(driver: &mut dyn DatabaseDriver, table: &str, name: &str) -> String {
    let query =
        driver.object_create_query(None, Some("app"), Some(table), name, ObjectType::Trigger);
    live::create_text(driver, query).await
}

/// Reads the CREATE text of a view of the schema `app`.
async fn view_text(driver: &mut dyn DatabaseDriver, name: &str, relation: RelationType) -> String {
    let query = driver.create_query(None, Some("app"), name, relation);
    live::create_text(driver, query).await
}

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_the_simple_path_names_the_types_of_a_read_and_of_a_write() {
    let Some((scratch, mut driver, _)) = Scratch::open("pg_types").await else {
        return;
    };
    let body = async move {
        let driver = driver.as_mut();
        let read = live::run(
            driver,
            "SELECT id, total, mood, qty, tags, note FROM app.orders ORDER BY id",
        )
        .await;
        // The server describes a column of a domain with the base type of
        // the domain.
        assert_eq!(
            type_names(&read),
            ["int4", "numeric", "mood", "int4", "_text", "text"]
        );
        assert_eq!(live::cell(&read, 0, 4).as_deref(), Some("{a,b}"));

        // A write is not prepared, so a type that is not built in shows the
        // number of its OID.
        let oid = live::run(driver, "SELECT 'app.mood'::regtype::oid").await;
        let mood_oid = live::cell(&oid, 0, 0).unwrap();
        let write = live::run(
            driver,
            "INSERT INTO app.orders (id, total, mood, qty, tags) \
             VALUES (3, 1, 'happy', 1, '{c}') RETURNING id, mood, tags",
        )
        .await;
        assert_eq!(type_names(&write), ["int4", mood_oid.as_str(), "_text"]);
        assert_eq!(write.results[0].rows.len(), 1);
    };
    live::with_cleanup(body, scratch.remove()).await;
}

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_a_read_past_the_row_limit_stops_the_server_and_the_next_statement_runs() {
    let Some((scratch, mut driver, _)) = Scratch::open("pg_limit").await else {
        return;
    };
    let body = async move {
        let driver = driver.as_mut();
        let options = ExecOptions {
            max_rows: 100,
            ..ExecOptions::default()
        };
        // Each round gives the signal of a late cancel a chance to stop the
        // statement after it.
        for _ in 0..5 {
            let started = Instant::now();
            let read = live::run_with(
                driver,
                "SELECT g FROM generate_series(1, 200000000) AS g",
                &options,
            )
            .await;
            assert_eq!(read.results[0].rows.len(), 100);
            assert!(read.results[0].truncated);
            assert!(started.elapsed() < Duration::from_secs(20));

            let next = live::run(driver, "SELECT pg_sleep(0.2), 7 AS seven").await;
            assert_eq!(live::cell(&next, 0, 1).as_deref(), Some("7"));
        }
    };
    live::with_cleanup(body, scratch.remove()).await;
}

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_the_cancel_handle_stops_a_statement_that_runs() {
    let Some((scratch, mut driver, _)) = Scratch::open("pg_cancel").await else {
        return;
    };
    let body = async move {
        let stop = driver.cancel_handle().expect("the driver can cancel");
        let request = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            stop.cancel().await
        });
        let started = Instant::now();
        let outcome = driver
            .execute_query("SELECT pg_sleep(30)", None, &ExecOptions::default())
            .await;
        assert!(outcome.is_err(), "the cancel stops the statement");
        assert!(started.elapsed() < Duration::from_secs(15));
        request.await.unwrap().unwrap();

        let next = live::run(driver.as_mut(), "SELECT 1").await;
        assert_eq!(live::cell(&next, 0, 0).as_deref(), Some("1"));
    };
    live::with_cleanup(body, scratch.remove()).await;
}

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_the_tree_and_the_snapshot_name_each_type_of_relation() {
    let Some((scratch, mut driver, _)) = Scratch::open("pg_relations").await else {
        return;
    };
    let database = scratch.name.clone();
    let body = async move {
        let driver = driver.as_mut();
        let tables = driver.list_tables(&database, Some("app")).await.unwrap();
        // The list sorts by the `relkind` letter and then by the name.
        assert_eq!(
            tables,
            [
                Table::new("host_file", RelationType::ForeignTable),
                Table::new("order_totals", RelationType::MaterializedView),
                Table::new("pending_totals", RelationType::MaterializedView),
                Table::new("events", RelationType::PartitionedTable),
                Table::table("lines"),
                Table::table("orders"),
                Table::view("big_orders"),
            ]
        );

        let partitions = driver
            .list_partitions(&database, Some("app"), "events")
            .await
            .unwrap();
        let values: Vec<&str> = partitions
            .partitions
            .iter()
            .map(|partition| partition.values.as_str())
            .collect();
        assert_eq!(
            values,
            [
                "app.events_2025 FOR VALUES FROM ('2025-01-01') TO ('2026-01-01')",
                "app.events_2026 FOR VALUES FROM ('2026-01-01') TO ('2027-01-01')",
            ]
        );
        assert!(!partitions.truncated);

        let columns = driver
            .list_columns(&database, Some("app"), "order_totals")
            .await
            .unwrap();
        let names: Vec<&str> = columns.iter().map(|column| column.name.as_str()).collect();
        assert_eq!(names, ["mood", "total"]);

        let snapshot = driver.schema_snapshot(&database, 10_000).await.unwrap();
        assert!(snapshot.complete);
        let in_app = |name: &str| {
            snapshot
                .relations
                .iter()
                .find(|relation| relation.schema.as_deref() == Some("app") && relation.name == name)
                .map(|relation| relation.relation_type)
        };
        assert_eq!(in_app("order_totals"), Some(RelationType::MaterializedView));
        assert_eq!(in_app("host_file"), Some(RelationType::ForeignTable));
        assert_eq!(in_app("events"), Some(RelationType::PartitionedTable));
        assert_eq!(in_app("events_2025"), None);
    };
    live::with_cleanup(body, scratch.remove()).await;
}

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_the_triggers_give_their_timing_their_events_and_their_state() {
    use TriggerEvent::{Delete, Insert, Truncate, Update};
    use TriggerTiming::{After, Before, InsteadOf};
    let Some((scratch, mut driver, _)) = Scratch::open("pg_triggers").await else {
        return;
    };
    let database = scratch.name.clone();
    let body = async move {
        let driver = driver.as_mut();
        // The foreign key of `lines` makes internal triggers on both tables,
        // and the list leaves them out.
        assert_eq!(
            driver
                .list_triggers(&database, Some("app"), "orders")
                .await
                .unwrap(),
            [
                trigger("a_before_write", Before, &[Insert, Update], true),
                trigger("b_after_delete", After, &[Delete], true),
                trigger("c_truncate", After, &[Truncate], true),
                trigger("d_disabled", After, &[Insert, Update, Delete], false),
            ]
        );
        assert_eq!(
            driver
                .list_triggers(&database, Some("app"), "lines")
                .await
                .unwrap(),
            []
        );
        assert_eq!(
            driver
                .list_triggers(&database, Some("app"), "big_orders")
                .await
                .unwrap(),
            [trigger(
                "v_instead",
                InsteadOf,
                &[Insert, Update, Delete],
                true
            )]
        );
        assert_eq!(
            driver
                .list_triggers(&database, Some("app"), "events")
                .await
                .unwrap(),
            [trigger("p_after_insert", After, &[Insert], true)]
        );
        assert_eq!(
            driver
                .list_triggers(&database, Some("app"), "host_file")
                .await
                .unwrap(),
            [trigger("f_before_insert", Before, &[Insert], true)]
        );
    };
    live::with_cleanup(body, scratch.remove()).await;
}

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_the_create_text_runs_again_after_a_drop() {
    let Some((scratch, mut driver, _)) = Scratch::open("pg_scripts").await else {
        return;
    };
    let body = async move {
        let driver = driver.as_mut();
        for (table, name) in [
            ("orders", "a_before_write"),
            ("orders", "c_truncate"),
            ("orders", "d_disabled"),
            ("big_orders", "v_instead"),
            ("events", "p_after_insert"),
            ("host_file", "f_before_insert"),
        ] {
            let text = trigger_text(driver, table, name).await;
            live::run(driver, &format!("DROP TRIGGER {name} ON app.{table}")).await;
            live::run(driver, &text).await;
            assert_eq!(trigger_text(driver, table, name).await, text);
        }

        // A drop of a view also drops its triggers, so the views come after
        // the triggers.
        for (name, relation, word) in [
            ("big_orders", RelationType::View, "VIEW"),
            (
                "order_totals",
                RelationType::MaterializedView,
                "MATERIALIZED VIEW",
            ),
            (
                "pending_totals",
                RelationType::MaterializedView,
                "MATERIALIZED VIEW",
            ),
        ] {
            let text = view_text(driver, name, relation).await;
            assert_eq!(text.ends_with("\nWITH NO DATA;"), name == "pending_totals");
            assert!(text.ends_with(';') && !text.ends_with(";;"), "{text}");
            assert_eq!(
                text.ends_with(
                    ";\n\nCREATE UNIQUE INDEX order_totals_mood \
                     ON app.order_totals USING btree (mood);"
                ),
                name == "order_totals",
                "{text}"
            );
            live::run(driver, &format!("DROP {word} app.{name}")).await;
            live::run(driver, &text).await;
            assert_eq!(view_text(driver, name, relation).await, text);
        }
        // The text of a materialized view without rows makes a view without
        // rows again.
        let populated = live::run(
            driver,
            "SELECT relname, relispopulated FROM pg_catalog.pg_class \
             WHERE relname IN ('order_totals', 'pending_totals') ORDER BY relname",
        )
        .await;
        assert_eq!(live::cell(&populated, 0, 1).as_deref(), Some("t"));
        assert_eq!(live::cell(&populated, 1, 1).as_deref(), Some("f"));
        // The text of a materialized view makes its unique index again, so
        // a concurrent refresh of the view runs.
        let index = live::run(
            driver,
            "SELECT i.indisunique FROM pg_catalog.pg_index AS i \
             JOIN pg_catalog.pg_class AS x ON x.oid = i.indexrelid \
             WHERE x.relname = 'order_totals_mood' \
             AND i.indrelid = 'app.order_totals'::regclass",
        )
        .await;
        assert_eq!(live::cell(&index, 0, 0).as_deref(), Some("t"));
        live::run(
            driver,
            "REFRESH MATERIALIZED VIEW CONCURRENTLY app.order_totals",
        )
        .await;
    };
    live::with_cleanup(body, scratch.remove()).await;
}

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_the_create_text_names_each_schema_under_any_search_path() {
    let Some((scratch, mut driver, server)) = Scratch::open("pg_path").await else {
        return;
    };
    let database = scratch.name.clone();
    let body = async move {
        let driver = driver.as_mut();
        live::run(driver, "SET search_path TO app").await;
        let trigger = trigger_text(driver, "orders", "a_before_write").await;
        let view = view_text(driver, "big_orders", RelationType::View).await;
        let matview = view_text(driver, "order_totals", RelationType::MaterializedView).await;
        assert!(trigger.contains(" ON app.orders "), "{trigger}");
        assert!(trigger.contains("EXECUTE FUNCTION app.pass()"), "{trigger}");
        assert!(
            view.starts_with("CREATE OR REPLACE VIEW app.big_orders AS"),
            "{view}"
        );
        assert!(view.contains("FROM app.orders"), "{view}");
        assert!(matview.starts_with("CREATE MATERIALIZED VIEW app.order_totals AS"));
        assert!(matview.contains("FROM app.orders"), "{matview}");
        // The read sets the search path for its own statement alone.
        let path = live::run(driver, "SHOW search_path").await;
        assert_eq!(live::cell(&path, 0, 0).as_deref(), Some("app"));

        // A session with the default search path runs each text again.
        let mut other = server.open(DbType::Postgres, Some(&database)).await;
        let other = other.as_mut();
        live::run(other, "DROP TRIGGER a_before_write ON app.orders").await;
        live::run(other, &trigger).await;
        live::run(other, "DROP VIEW app.big_orders").await;
        live::run(other, &view).await;
        live::run(other, "DROP MATERIALIZED VIEW app.order_totals").await;
        live::run(other, &matview).await;
        assert_eq!(
            trigger_text(other, "orders", "a_before_write").await,
            trigger
        );
        assert_eq!(
            view_text(other, "big_orders", RelationType::View).await,
            view
        );
    };
    live::with_cleanup(body, scratch.remove()).await;
}

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_a_replica_trigger_is_not_enabled_and_an_always_trigger_is_enabled() {
    use TriggerEvent::{Delete, Truncate};
    use TriggerTiming::After;
    let Some((scratch, mut driver, _)) = Scratch::open("pg_replica").await else {
        return;
    };
    let database = scratch.name.clone();
    let body = async move {
        let driver = driver.as_mut();
        let replica = "ALTER TABLE app.orders ENABLE REPLICA TRIGGER b_after_delete";
        live::run(driver, replica).await;
        let always = "ALTER TABLE app.orders ENABLE ALWAYS TRIGGER c_truncate";
        live::run(driver, always).await;
        let triggers = driver
            .list_triggers(&database, Some("app"), "orders")
            .await
            .unwrap();
        assert_eq!(
            triggers[1],
            Trigger {
                replica: true,
                ..trigger("b_after_delete", After, &[Delete], false)
            }
        );
        assert_eq!(triggers[2], trigger("c_truncate", After, &[Truncate], true));
    };
    live::with_cleanup(body, scratch.remove()).await;
}
