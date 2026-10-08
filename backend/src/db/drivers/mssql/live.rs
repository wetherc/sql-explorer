//! The tests of the MS SQL Server driver against a live server. The variable
//! `SQLX_LIVE_MSSQL` names the server, see [`crate::db::drivers::live`].

use crate::db::drivers::live::{self, Server};
use crate::db::drivers::{add_snapshot_column, add_snapshot_relation, DatabaseDriver};
use crate::db::{
    ObjectType, RelationType, SchemaSnapshot, Table, Trigger, TriggerEvent, TriggerTiming,
};
use crate::storage::DbType;
use std::time::Duration;

const FIXTURE: &str = include_str!("../../../../live/fixtures/mssql.sql");

/// Folds the rows of a whole snapshot again under a lower bound, as the
/// driver folds the rows of the server. A relation with no column stands
/// for one row with no column.
fn fold_with_bound(whole: &SchemaSnapshot, bound: usize) -> SchemaSnapshot {
    let mut snapshot = SchemaSnapshot {
        database: whole.database.clone(),
        complete: true,
        ..SchemaSnapshot::default()
    };
    'rows: for relation in &whole.relations {
        let schema = relation.schema.clone();
        let name = relation.name.clone();
        if relation.columns.is_empty() {
            let kept =
                add_snapshot_relation(&mut snapshot, bound, schema, name, relation.relation_type);
            if !kept {
                break;
            }
            continue;
        }
        for column in &relation.columns {
            let kept = add_snapshot_column(
                &mut snapshot,
                bound,
                schema.clone(),
                name.clone(),
                relation.relation_type,
                column.clone(),
            );
            if !kept {
                break 'rows;
            }
        }
    }
    snapshot
}

/// The databases of one test, with the session that made them.
struct Scratch {
    admin: Box<dyn DatabaseDriver>,
    names: Vec<String>,
}

impl Scratch {
    /// Makes a database with the fixture and opens a driver on it. A server
    /// that is not configured gives `None`.
    async fn open(tag: &str) -> Option<(Scratch, Box<dyn DatabaseDriver>, Server)> {
        let server = live::server("SQLX_LIVE_MSSQL")?;
        let mut admin = server.open(DbType::Mssql, Some("master")).await;
        let name = live::unique_name(tag);
        live::run(admin.as_mut(), &format!("CREATE DATABASE {name}")).await;
        let mut driver = server.open(DbType::Mssql, Some(&name)).await;
        let scratch = Scratch {
            admin,
            names: vec![name],
        };
        if let Err(error) = live::load_fixture(driver.as_mut(), FIXTURE).await {
            drop(driver);
            scratch.remove().await;
            panic!("the fixture failed: {error}");
        }
        Some((scratch, driver, server))
    }

    /// Makes one more database that the cleanup removes.
    async fn add_database(&mut self, tag: &str) -> String {
        let name = live::unique_name(tag);
        live::run(self.admin.as_mut(), &format!("CREATE DATABASE {name}")).await;
        self.names.push(name.clone());
        name
    }

    fn name(&self) -> String {
        self.names[0].clone()
    }

    /// Removes the databases, and ends each session that still uses them.
    async fn remove(mut self) {
        for name in &self.names {
            let sql = format!(
                "ALTER DATABASE {name} SET SINGLE_USER WITH ROLLBACK IMMEDIATE;\n\
                 DROP DATABASE {name};"
            );
            live::run(self.admin.as_mut(), &sql).await;
        }
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

#[tokio::test]
#[ignore = "needs a live MS SQL Server"]
async fn live_binary_values_show_in_hexadecimal() {
    let Some((scratch, mut driver, _)) = Scratch::open("ms_binary").await else {
        return;
    };
    let body = async move {
        let response = live::run(
            driver.as_mut(),
            "SELECT b, vb, img, rv, CAST(0x AS varbinary(10)) FROM dbo.Orders ORDER BY id",
        )
        .await;
        let row = |index: usize| -> Vec<Option<String>> {
            (0..5)
                .map(|column| live::cell(&response, index, column))
                .collect()
        };
        let first = row(0);
        assert_eq!(first[0].as_deref(), Some("0x0A1B0000"));
        assert_eq!(first[1].as_deref(), Some("0xABCDEF"));
        assert_eq!(first[2].as_deref(), Some("0x01"));
        let version = first[3].clone().unwrap();
        assert!(
            version.starts_with("0x") && version.len() == 18,
            "{version}"
        );
        assert_eq!(first[4].as_deref(), Some("0x"));
        let second = row(1);
        assert_eq!(second[0].as_deref(), Some("0x00000000"));
        assert_eq!(second[1].as_deref(), Some("0x"));
        assert_eq!(second[2], None);
    };
    live::with_cleanup(body, scratch.remove()).await;
}

#[tokio::test]
#[ignore = "needs a live MS SQL Server"]
async fn live_a_synonym_lists_its_target_and_its_create_text_runs_again() {
    let Some((mut scratch, mut driver, _)) = Scratch::open("ms_synonyms").await else {
        return;
    };
    let database = scratch.name();
    let other = scratch.add_database("ms_other").await;
    let body = async move {
        let driver = driver.as_mut();
        live::run(
            driver,
            &format!(
                "CREATE TABLE {other}.dbo.Remote (id int PRIMARY KEY);\n\
                 INSERT INTO {other}.dbo.Remote VALUES (1), (2);\n\
                 CREATE SYNONYM dbo.RemoteSyn FOR {other}.dbo.Remote;"
            ),
        )
        .await;

        let tables = driver.list_tables(&database, Some("dbo")).await.unwrap();
        let synonyms: Vec<&Table> = tables
            .iter()
            .filter(|table| table.relation_type == RelationType::Synonym)
            .collect();
        // The server keeps the target with brackets around each part, also
        // when the CREATE statement wrote no brackets.
        assert_eq!(
            synonyms,
            [
                &Table::synonym("OrdersSyn", "[dbo].[Orders]"),
                &Table::synonym("RemoteSyn", format!("[{other}].[dbo].[Remote]")),
            ]
        );
        let in_hr = driver.list_tables(&database, Some("hr")).await.unwrap();
        assert!(in_hr.contains(&Table::synonym("PeopleSyn", "[hr].[People]")));

        for (schema, name) in [
            ("dbo", "OrdersSyn"),
            ("dbo", "RemoteSyn"),
            ("hr", "PeopleSyn"),
        ] {
            let read = |driver: &mut dyn DatabaseDriver| {
                driver.create_query(Some(&database), Some(schema), name, RelationType::Synonym)
            };
            let query = read(driver);
            let text = live::create_text(driver, query).await;
            live::run(driver, &format!("DROP SYNONYM {schema}.{name}")).await;
            live::run(driver, &text).await;
            let query = read(driver);
            assert_eq!(live::create_text(driver, query).await, text);
        }
        let count = live::run(driver, "SELECT COUNT(*) FROM dbo.RemoteSyn").await;
        assert_eq!(live::cell(&count, 0, 0).as_deref(), Some("2"));

        // A target in another database gives the snapshot the name of the
        // synonym and no column.
        let snapshot = driver.schema_snapshot(&database, 10_000).await.unwrap();
        let remote = snapshot
            .relations
            .iter()
            .find(|relation| relation.name == "RemoteSyn")
            .expect("the synonym is in the snapshot");
        assert_eq!(remote.relation_type, RelationType::Synonym);
        assert!(remote.columns.is_empty());
    };
    live::with_cleanup(body, scratch.remove()).await;
}

#[tokio::test]
#[ignore = "needs a live MS SQL Server"]
async fn live_the_triggers_of_a_relation_leave_out_a_trigger_of_the_database() {
    use TriggerEvent::{Delete, Insert, Update};
    use TriggerTiming::{After, InsteadOf};
    let Some((scratch, mut driver, _)) = Scratch::open("ms_triggers").await else {
        return;
    };
    let database = scratch.name();
    let body = async move {
        let driver = driver.as_mut();
        let mut list = Vec::new();
        for (schema, table) in [
            ("dbo", "Orders"),
            ("dbo", "BigOrders"),
            ("dbo", "Lines"),
            ("hr", "People"),
        ] {
            list.push(
                driver
                    .list_triggers(&database, Some(schema), table)
                    .await
                    .unwrap(),
            );
        }
        assert_eq!(
            list,
            [
                vec![
                    trigger("trg_delete", After, &[Delete], true),
                    trigger("trg_disabled", After, &[Insert, Update, Delete], false),
                    trigger("trg_write", After, &[Insert, Update], true),
                ],
                vec![trigger("trg_instead", InsteadOf, &[Insert, Delete], true)],
                vec![],
                vec![trigger("trg_people", After, &[Update], true)],
            ]
        );

        for (schema, table, name) in [
            ("dbo", "Orders", "trg_write"),
            ("dbo", "BigOrders", "trg_instead"),
            ("hr", "People", "trg_people"),
        ] {
            let read = |driver: &mut dyn DatabaseDriver| {
                driver.object_create_query(
                    Some(&database),
                    Some(schema),
                    Some(table),
                    name,
                    ObjectType::Trigger,
                )
            };
            let query = read(driver);
            let text = live::create_text(driver, query).await;
            live::run(driver, &format!("DROP TRIGGER {schema}.{name}")).await;
            live::run(driver, &text).await;
            let query = read(driver);
            assert_eq!(live::create_text(driver, query).await, text);
        }

        // A drop of a view also drops its triggers, so the view comes after
        // the triggers.
        let read = |driver: &mut dyn DatabaseDriver| {
            driver.create_query(
                Some(&database),
                Some("dbo"),
                "BigOrders",
                RelationType::View,
            )
        };
        let query = read(driver);
        let text = live::create_text(driver, query).await;
        live::run(driver, "DROP VIEW dbo.BigOrders").await;
        live::run(driver, &text).await;
        let query = read(driver);
        assert_eq!(live::create_text(driver, query).await, text);

        // A target with no schema resolves through the default schema of the
        // user, and a target that does not exist gives no column.
        live::run(
            driver,
            "CREATE SYNONYM dbo.LinesSyn FOR Lines;\n\
             CREATE SYNONYM dbo.GoneSyn FOR dbo.Gone;",
        )
        .await;
        let snapshot = driver.schema_snapshot(&database, 10_000).await.unwrap();
        assert!(snapshot.complete);
        assert!(snapshot
            .relations
            .iter()
            .any(|relation| relation.name == "Orders" && relation.columns.len() == 6));
        let synonym = |schema: &str, name: &str| {
            snapshot
                .relations
                .iter()
                .find(|relation| {
                    relation.schema.as_deref() == Some(schema) && relation.name == name
                })
                .unwrap_or_else(|| panic!("{schema}.{name} is not in the snapshot"))
        };
        let orders = synonym("dbo", "OrdersSyn");
        assert_eq!(orders.relation_type, RelationType::Synonym);
        let names: Vec<&str> = orders.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["id", "total", "b", "vb", "rv", "img"]);
        assert_eq!(orders.columns[1].data_type, "money");
        let people = synonym("hr", "PeopleSyn");
        let names: Vec<&str> = people.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["id", "name"]);
        assert_eq!(synonym("dbo", "LinesSyn").columns.len(), 2);
        assert!(synonym("dbo", "GoneSyn").columns.is_empty());
        // Each relation has one record, so the rows of a synonym arrive
        // together.
        let mut keys: Vec<_> = snapshot
            .relations
            .iter()
            .map(|relation| (relation.schema.clone(), relation.name.clone()))
            .collect();
        let count = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), count);

        // The server cuts the rows at the bound, and the snapshot of each
        // bound is the one that the fold makes from every row.
        let total = snapshot.column_count;
        for bound in [0, 1, 7, total - 1, total] {
            let cut = driver.schema_snapshot(&database, bound).await.unwrap();
            assert_eq!(cut, fold_with_bound(&snapshot, bound), "bound {bound}");
        }
    };
    live::with_cleanup(body, scratch.remove()).await;
}

#[tokio::test]
#[ignore = "needs a live MS SQL Server"]
async fn live_identity_computed_and_rowversion_columns_are_marked() {
    let Some((scratch, mut driver, _)) = Scratch::open("ms_generated").await else {
        return;
    };
    let database = scratch.name();
    let body = async move {
        live::run(
            driver.as_mut(),
            "CREATE TABLE dbo.Filled (\
                 a int IDENTITY PRIMARY KEY, \
                 c int, \
                 d AS (c * 2), \
                 v rowversion)",
        )
        .await;
        let columns = driver
            .list_columns(&database, Some("dbo"), "Filled")
            .await
            .unwrap();
        let marks: Vec<bool> = columns.iter().map(|column| column.is_generated).collect();
        assert_eq!(marks, [true, false, true, true]);
    };
    live::with_cleanup(body, scratch.remove()).await;
}

#[tokio::test]
#[ignore = "needs a live MS SQL Server"]
async fn live_the_column_types_name_the_digits_of_their_fraction_and_a_short_float() {
    let Some((scratch, mut driver, _)) = Scratch::open("ms_types").await else {
        return;
    };
    let database = scratch.name();
    let body = async move {
        live::run(
            driver.as_mut(),
            "CREATE TABLE dbo.Timed (\
                 a datetime2(3), \
                 b time(0), \
                 c datetimeoffset, \
                 d float(10), \
                 e float, \
                 f datetime)",
        )
        .await;
        let columns = driver
            .list_columns(&database, Some("dbo"), "Timed")
            .await
            .unwrap();
        let types: Vec<&str> = columns
            .iter()
            .map(|column| column.data_type.as_str())
            .collect();
        assert_eq!(
            types,
            [
                "datetime2(3)",
                "time(0)",
                "datetimeoffset(7)",
                "real",
                "float",
                "datetime"
            ]
        );
    };
    live::with_cleanup(body, scratch.remove()).await;
}

/// A read of 200,000 numbers in order. The rows fill the buffers of the
/// connection, so the server must wait while the read is paused.
const NUMBERS: &str = "SELECT TOP (200000) ROW_NUMBER() OVER (ORDER BY (SELECT NULL)) AS n \
     FROM sys.all_objects a CROSS JOIN sys.all_objects b ORDER BY n";

#[tokio::test]
#[ignore = "needs a live MS SQL Server"]
async fn live_a_paused_read_exports_every_row_once() {
    let Some(server) = live::server("SQLX_LIVE_MSSQL") else {
        return;
    };
    let driver = server.open(DbType::Mssql, Some("master")).await;
    let paused = live::pause_read(driver, NUMBERS, 100, Duration::from_secs(60)).await;
    assert_eq!(live::numbers(&paused.grid), (1..=100).collect::<Vec<_>>());
    // The server waits while the read is paused.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let rows = live::export_paused(&paused.read, usize::MAX).await.unwrap();
    assert_eq!(live::numbers(&rows), (1..=200_000).collect::<Vec<_>>());
    let response = live::run_after(&paused.session, "SELECT 1 AS one").await;
    assert_eq!(live::cell(&response, 0, 0).as_deref(), Some("1"));
}

#[tokio::test]
#[ignore = "needs a live MS SQL Server"]
async fn live_a_released_read_frees_its_session() {
    let Some(server) = live::server("SQLX_LIVE_MSSQL") else {
        return;
    };
    let driver = server.open(DbType::Mssql, Some("master")).await;
    let paused = live::pause_read(driver, NUMBERS, 100, Duration::from_secs(60)).await;
    let session = paused.session.clone();
    drop(paused);
    let response = live::run_after(&session, "SELECT 2 AS two").await;
    assert_eq!(live::cell(&response, 0, 0).as_deref(), Some("2"));
}

#[tokio::test]
#[ignore = "needs a live MS SQL Server"]
async fn live_the_end_of_the_pause_releases_the_read() {
    let Some(server) = live::server("SQLX_LIVE_MSSQL") else {
        return;
    };
    let driver = server.open(DbType::Mssql, Some("master")).await;
    let paused = live::pause_read(driver, NUMBERS, 100, Duration::from_secs(1)).await;
    let response = live::run_after(&paused.session, "SELECT 3 AS three").await;
    assert_eq!(live::cell(&response, 0, 0).as_deref(), Some("3"));
    assert!(live::export_paused(&paused.read, usize::MAX).await.is_err());
}

#[tokio::test]
#[ignore = "needs a live MS SQL Server"]
async fn live_the_report_names_the_session_that_blocks_a_read() {
    let Some((scratch, mut driver, server)) = Scratch::open("ms_block").await else {
        return;
    };
    let name = scratch.name();
    let body = async move {
        live::run(
            driver.as_mut(),
            "CREATE TABLE dbo.LockProbe (id int PRIMARY KEY, v int);\n\
             INSERT INTO dbo.LockProbe VALUES (1, 1);",
        )
        .await;
        let holder = server.open(DbType::Mssql, Some(&name)).await;
        let waiter = server.open(DbType::Mssql, Some(&name)).await;
        let scene = live::LockScene {
            session_id: "SELECT @@SPID",
            lock: "BEGIN TRANSACTION; UPDATE dbo.LockProbe SET v = 2 WHERE id = 1;",
            wait: "SELECT v FROM dbo.LockProbe WHERE id = 1",
            release: "ROLLBACK",
        };
        let locked = live::report_during_wait(holder, waiter, driver.as_mut(), &scene).await;
        let report = &locked.report;
        let wait = report
            .sessions
            .iter()
            .find(|row| row.waiting_session == locked.waiter)
            .unwrap_or_else(|| panic!("no wait in {report:?}"));
        assert_eq!(wait.blocking_session, locked.holder);
        assert_eq!(wait.lock_mode.as_deref(), Some("S"));
        let object = format!("[{name}].[dbo].[LockProbe]");
        assert_eq!(wait.object.as_deref(), Some(object.as_str()));
        assert!(wait
            .blocking_statement
            .as_deref()
            .is_some_and(|text| text.contains("UPDATE dbo.LockProbe")));
        assert!(
            wait.waiting_statement
                .as_deref()
                .is_some_and(|text| text.contains("[LockProbe]")),
            "{wait:?}"
        );
        assert_eq!(wait.blocking_status.as_deref(), Some("sleeping"));
        assert!(report
            .open_transactions
            .iter()
            .any(|row| row.session == locked.holder));
        assert!(report.notes.is_empty());
    };
    live::with_cleanup(body, scratch.remove()).await;
}

#[tokio::test]
#[ignore = "needs a live MS SQL Server"]
async fn live_a_login_without_server_state_gets_a_note() {
    let Some(server) = live::server("SQLX_LIVE_MSSQL") else {
        return;
    };
    let mut admin = server.open(DbType::Mssql, Some("master")).await;
    let login = live::unique_name("nostate");
    let password = "Live#NoState2026";
    live::run(
        admin.as_mut(),
        &format!("CREATE LOGIN {login} WITH PASSWORD = '{password}', CHECK_POLICY = OFF"),
    )
    .await;
    let limited = Server {
        user: login.clone(),
        password: password.to_string(),
        ..server.clone()
    };
    let body = async move {
        let mut driver = limited.open(DbType::Mssql, Some("master")).await;
        let report = driver.blocking_sessions().await.unwrap();
        assert!(report.sessions.is_empty());
        assert!(report.notes[0].contains("VIEW SERVER STATE"), "{report:?}");
    };
    let cleanup = async move {
        live::run(admin.as_mut(), &format!("DROP LOGIN {login}")).await;
    };
    live::with_cleanup(body, cleanup).await;
}
