//! The tests of the PostgreSQL driver against a live server. The variable
//! `SQLX_LIVE_PG` names the server, see [`crate::db::drivers::live`].

use crate::db::drivers::live::{self, Server};
use crate::db::drivers::DatabaseDriver;
use crate::db::{
    Constraint, ConstraintType, ExecOptions, ObjectType, RelationType, Table, Trigger,
    TriggerEvent, TriggerTiming,
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
        update_columns: Vec::new(),
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
async fn live_a_read_inside_a_block_stops_at_the_row_limit_and_keeps_the_block() {
    let Some((scratch, mut driver, _)) = Scratch::open("pg_block").await else {
        return;
    };
    let body = async move {
        let driver = driver.as_mut();
        let options = ExecOptions {
            max_rows: 100,
            ..ExecOptions::default()
        };
        live::run(driver, "CREATE TABLE app.kept (n int)").await;
        live::run(driver, "BEGIN; INSERT INTO app.kept VALUES (1)").await;

        // The cursor stops the read at the limit, so a set of two hundred
        // million rows ends at once.
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

        // The path with parameters reads through a portal in the block.
        let params = vec![crate::db::QueryParam {
            value: serde_json::json!(1),
        }];
        let started = Instant::now();
        let read = driver
            .execute_query(
                "SELECT g FROM generate_series(1, 200000000) AS g WHERE $1::int = 1",
                Some(&params),
                &options,
            )
            .await
            .unwrap();
        assert_eq!(read.results[0].rows.len(), 100);
        assert!(read.results[0].truncated);
        assert!(started.elapsed() < Duration::from_secs(20));

        // A read with a row lock runs whole and gives the same rows.
        let read = live::run_with(driver, "SELECT n FROM app.kept FOR UPDATE", &options).await;
        assert_eq!(live::cell(&read, 0, 0).as_deref(), Some("1"));

        // The block stays open, and its work stays until the user ends it.
        assert!(driver.holds_open_transaction().await.unwrap());
        live::run(driver, "COMMIT").await;
        let count = live::run(driver, "SELECT count(*) FROM app.kept").await;
        assert_eq!(live::cell(&count, 0, 0).as_deref(), Some("1"));

        // An error of a read inside a block aborts the block, as the
        // statement alone would, and the user ends it.
        live::run(driver, "BEGIN").await;
        let error = driver
            .execute_query("SELECT 1,\n  nope FROM app.kept", None, &options)
            .await
            .unwrap_err();
        let payload = error.to_payload();
        assert_eq!((payload.line, payload.column), (Some(2), Some(3)));
        assert!(driver.holds_open_transaction().await.unwrap());
        live::run(driver, "ROLLBACK").await;
        assert!(!driver.holds_open_transaction().await.unwrap());
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
        // and the list leaves them out. The list of `lines` also leaves out
        // the constraint trigger, which the list of the constraints gives.
        // The columns of `UPDATE OF note, total` keep the order of the
        // clause, which is the reverse of the order of the table.
        assert_eq!(
            driver
                .list_triggers(&database, Some("app"), "orders")
                .await
                .unwrap(),
            [
                Trigger {
                    update_columns: vec!["note".into(), "total".into()],
                    ..trigger("a_before_write", Before, &[Insert, Update], true)
                },
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
        let constraints = driver
            .list_constraints(&database, Some("app"), "lines")
            .await
            .unwrap();
        assert!(
            constraints.contains(&Constraint {
                name: "g_check_line".into(),
                constraint_type: ConstraintType::Trigger,
                columns: Vec::new(),
                detail: Some("TRIGGER DEFERRABLE INITIALLY DEFERRED".into()),
            }),
            "{constraints:?}"
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

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_the_create_text_makes_a_trigger_with_the_same_state_again() {
    let Some((scratch, mut driver, _)) = Scratch::open("pg_state").await else {
        return;
    };
    let body = async move {
        let driver = driver.as_mut();
        live::run(
            driver,
            "ALTER TABLE app.orders ENABLE REPLICA TRIGGER b_after_delete; \
             ALTER TABLE app.orders ENABLE ALWAYS TRIGGER c_truncate",
        )
        .await;
        for (name, letter, statement) in [
            ("a_before_write", "O", None),
            ("b_after_delete", "R", Some("ENABLE REPLICA TRIGGER")),
            ("c_truncate", "A", Some("ENABLE ALWAYS TRIGGER")),
            ("d_disabled", "D", Some("DISABLE TRIGGER")),
        ] {
            let text = trigger_text(driver, "orders", name).await;
            assert!(text.starts_with("CREATE TRIGGER "), "{text}");
            assert!(!text.contains(";;"), "{text}");
            match statement {
                Some(statement) => assert!(
                    text.ends_with(&format!(";\nALTER TABLE app.orders {statement} {name};")),
                    "{text}"
                ),
                None => assert!(!text.contains("ALTER TABLE"), "{text}"),
            }
            live::run(driver, &format!("DROP TRIGGER {name} ON app.orders")).await;
            live::run(driver, &text).await;
            let state = live::run(
                driver,
                &format!(
                    "SELECT tgenabled::text FROM pg_catalog.pg_trigger \
                     WHERE tgrelid = 'app.orders'::regclass AND tgname = '{name}'"
                ),
            )
            .await;
            assert_eq!(live::cell(&state, 0, 0).as_deref(), Some(letter), "{name}");
            assert_eq!(trigger_text(driver, "orders", name).await, text);
        }
    };
    live::with_cleanup(body, scratch.remove()).await;
}

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_identity_and_generated_columns_are_marked() {
    let Some((scratch, mut driver, _)) = Scratch::open("pg_generated").await else {
        return;
    };
    let database = scratch.name.clone();
    let body = async move {
        let driver = driver.as_mut();
        live::run(
            driver,
            "CREATE TABLE app.filled (\
                 a int GENERATED ALWAYS AS IDENTITY PRIMARY KEY, \
                 b int GENERATED BY DEFAULT AS IDENTITY, \
                 c int, \
                 d int GENERATED ALWAYS AS (c * 2) STORED)",
        )
        .await;
        let columns = driver
            .list_columns(&database, Some("app"), "filled")
            .await
            .unwrap();
        let marks: Vec<bool> = columns.iter().map(|column| column.is_generated).collect();
        assert_eq!(marks, [true, true, false, true]);
    };
    live::with_cleanup(body, scratch.remove()).await;
}

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_a_failed_script_keeps_its_notices_and_names_the_place_of_the_error() {
    let Some((scratch, mut driver, _)) = Scratch::open("pg_notice").await else {
        return;
    };
    let body = async move {
        let driver = driver.as_mut();
        let mut sink = crate::db::sink::BufferSink::new(100);
        let error = driver
            .execute_stream(
                "DO $$ BEGIN RAISE NOTICE 'hello'; END $$;\n\
                 SELECT 1;\n  SELECT nope\n  FROM app.orders",
                None,
                &ExecOptions::default(),
                &mut sink,
            )
            .await
            .unwrap_err();
        let payload = error.to_payload();
        assert_eq!((payload.line, payload.column), (Some(3), Some(10)));
        let response = sink.into_response(crate::db::sink::RunSummary {
            rows_affected: None,
            elapsed_ms: 0,
            stats: None,
        });
        assert!(response
            .messages
            .iter()
            .any(|message| message.text == "hello"));

        // A JSON value keeps the text of the server, and a bytea value shows
        // in hexadecimal, on the path with parameters as on the simple one.
        let params = vec![crate::db::QueryParam {
            value: serde_json::json!(1),
        }];
        let response = driver
            .execute_query(
                "SELECT '{\"b\": 1, \"a\": 2}'::json, '\\x6869'::bytea, $1::int",
                Some(&params),
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            live::cell(&response, 0, 0).as_deref(),
            Some("{\"b\": 1, \"a\": 2}")
        );
        assert_eq!(live::cell(&response, 0, 1).as_deref(), Some("\\x6869"));
    };
    live::with_cleanup(body, scratch.remove()).await;
}

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_a_read_past_the_row_limit_keeps_the_rows_that_its_function_inserts() {
    let Some((scratch, mut driver, _)) = Scratch::open("pg_keep").await else {
        return;
    };
    let body = async move {
        let driver = driver.as_mut();
        live::run(
            driver,
            "CREATE TABLE app.made (n int);\n\
             CREATE FUNCTION app.make() RETURNS SETOF int LANGUAGE plpgsql AS $$\n\
             BEGIN\n\
               RETURN QUERY INSERT INTO app.made SELECT g FROM generate_series(1, 5000) AS g\n\
                 RETURNING n;\n\
             END $$",
        )
        .await;
        let options = ExecOptions {
            max_rows: 100,
            ..ExecOptions::default()
        };
        let read = live::run_with(driver, "SELECT * FROM app.make()", &options).await;
        assert_eq!(read.results[0].rows.len(), 100);
        assert!(read.results[0].truncated);
        // The rows keep the text form of the simple protocol.
        assert_eq!(live::cell(&read, 0, 0).as_deref(), Some("1"));
        let count = live::run(driver, "SELECT count(*) FROM app.made").await;
        assert_eq!(live::cell(&count, 0, 0).as_deref(), Some("5000"));

        // The path with parameters reads through a portal and keeps the
        // rows as well.
        let params = vec![crate::db::QueryParam {
            value: serde_json::json!(1),
        }];
        let read = driver
            .execute_query(
                "SELECT * FROM app.make() WHERE $1::int = 1",
                Some(&params),
                &options,
            )
            .await
            .unwrap();
        assert_eq!(read.results[0].rows.len(), 100);
        assert!(read.results[0].truncated);
        let count = live::run(driver, "SELECT count(*) FROM app.made").await;
        assert_eq!(live::cell(&count, 0, 0).as_deref(), Some("10000"));
        assert!(!driver.holds_open_transaction().await.unwrap());

        // An error of the statement names its place in the text, and the
        // session leaves the transaction of the read.
        let error = driver
            .execute_query("SELECT 1,\n  nope FROM app.made", None, &options)
            .await
            .unwrap_err();
        let payload = error.to_payload();
        assert_eq!((payload.line, payload.column), (Some(2), Some(3)));
        assert!(!driver.holds_open_transaction().await.unwrap());
    };
    live::with_cleanup(body, scratch.remove()).await;
}

/// A read of 1,000,000 numbers in order, from 1, through a cursor.
const NUMBERS: &str = "SELECT n FROM generate_series(1, 1000000) AS n";

/// Opens a driver on the server, or gives `None` when no server is set.
async fn pg_driver() -> Option<Box<dyn DatabaseDriver>> {
    let server = live::server("SQLX_LIVE_PG")?;
    Some(server.open(DbType::Postgres, Some("postgres")).await)
}

/// Pauses a read on the session, exports every row in order, and runs a
/// second statement on the session, which then has no open transaction.
async fn exports_every_row(paused: live::Paused, total: i64) {
    assert_eq!(live::numbers(&paused.grid), (1..=100).collect::<Vec<_>>());
    tokio::time::sleep(Duration::from_secs(2)).await;
    let rows = live::export_paused(&paused.read, usize::MAX).await.unwrap();
    assert_eq!(live::numbers(&rows), (1..=total).collect::<Vec<_>>());
    let mut driver = paused.session.driver.lock().await;
    assert!(!driver.holds_open_transaction().await.unwrap());
}

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_a_paused_cursor_exports_every_row_once() {
    let Some(driver) = pg_driver().await else {
        return;
    };
    let mut driver = driver;
    let pid = live::run(driver.as_mut(), "SELECT pg_backend_pid()").await;
    let pid = live::cell(&pid, 0, 0).unwrap();
    let paused = live::pause_read(driver, NUMBERS, 100, Duration::from_secs(60)).await;
    // The fetch ends at the row that made the read pause, so the session
    // waits idle in its transaction.
    let server = live::server("SQLX_LIVE_PG").unwrap();
    let mut other = server.open(DbType::Postgres, Some("postgres")).await;
    let states = live::run(
        other.as_mut(),
        &format!(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE pid = {pid} AND state = 'idle in transaction'"
        ),
    )
    .await;
    assert_eq!(live::cell(&states, 0, 0).as_deref(), Some("1"));
    exports_every_row(paused, 1_000_000).await;
}

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_a_paused_portal_exports_every_row_once() {
    let Some(driver) = pg_driver().await else {
        return;
    };
    let params = vec![crate::db::QueryParam {
        value: serde_json::json!(300_000),
    }];
    let paused = live::pause_read_with(
        driver,
        "SELECT n FROM generate_series(1, $1::int) AS n",
        Some(params),
        100,
        Duration::from_secs(60),
    )
    .await;
    exports_every_row(paused, 300_000).await;
}

/// Pauses a read in a block of the user, exports every row in order, and
/// ends the block.
async fn exports_in_a_block(setup: &str, query: &str) {
    let Some(mut driver) = pg_driver().await else {
        return;
    };
    live::run(driver.as_mut(), setup).await;
    let paused = live::pause_read(driver, query, 100, Duration::from_secs(60)).await;
    assert_eq!(live::numbers(&paused.grid), (1..=100).collect::<Vec<_>>());
    let rows = live::export_paused(&paused.read, usize::MAX).await.unwrap();
    assert_eq!(live::numbers(&rows), (1..=200_000).collect::<Vec<_>>());
    live::run_after(&paused.session, "COMMIT").await;
}

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_a_paused_cursor_in_a_block_of_the_user_exports_every_row_once() {
    exports_in_a_block(
        "BEGIN",
        "SELECT n FROM generate_series(1, 200000) AS n ORDER BY n",
    )
    .await;
}

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_a_paused_walk_exports_every_row_once() {
    // A read with FOR UPDATE inside a block of the user keeps the walk.
    exports_in_a_block(
        "BEGIN; CREATE TEMP TABLE walked AS SELECT n FROM generate_series(1, 200000) AS n",
        "SELECT n FROM walked ORDER BY n FOR UPDATE",
    )
    .await;
}

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_a_released_cursor_frees_its_session() {
    let Some(driver) = pg_driver().await else {
        return;
    };
    let paused = live::pause_read(driver, NUMBERS, 100, Duration::from_secs(60)).await;
    let session = paused.session.clone();
    drop(paused);
    let response = live::run_after(&session, "SELECT 2 AS two").await;
    assert_eq!(live::cell(&response, 0, 0).as_deref(), Some("2"));
}

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_the_end_of_the_pause_releases_the_cursor() {
    let Some(driver) = pg_driver().await else {
        return;
    };
    let paused = live::pause_read(driver, NUMBERS, 100, Duration::from_secs(1)).await;
    let response = live::run_after(&paused.session, "SELECT 3 AS three").await;
    assert_eq!(live::cell(&response, 0, 0).as_deref(), Some("3"));
    assert!(live::export_paused(&paused.read, usize::MAX).await.is_err());
}

#[tokio::test]
#[ignore = "needs a live PostgreSQL server"]
async fn live_a_session_that_the_server_ends_during_a_pause_gives_a_clear_error() {
    let Some(mut driver) = pg_driver().await else {
        return;
    };
    live::run(
        driver.as_mut(),
        "SET idle_in_transaction_session_timeout = '1s'",
    )
    .await;
    let paused = live::pause_read(driver, NUMBERS, 100, Duration::from_secs(60)).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let error = live::export_paused(&paused.read, usize::MAX)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("closed the session while the read was paused"),
        "{error}"
    );
}
