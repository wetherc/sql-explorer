//! The tests of the MySQL driver against a live MySQL server and a live
//! MariaDB server. The variables `SQLX_LIVE_MYSQL` and `SQLX_LIVE_MARIADB`
//! name the servers, see [`crate::db::drivers::live`].

use crate::db::drivers::live::{self, Server};
use crate::db::drivers::DatabaseDriver;
use crate::db::{
    ObjectType, RelationType, ScheduledEvent, Table, Trigger, TriggerEvent, TriggerTiming,
};
use crate::storage::DbType;
use std::time::Duration;

const FIXTURE: &str = include_str!("../../../../live/fixtures/mysql.sql");

/// The database of one test, with the session that made it.
struct Scratch {
    admin: Box<dyn DatabaseDriver>,
    name: String,
}

impl Scratch {
    /// Makes a database with the fixture on the server of the variable and
    /// opens a driver on it. A server that is not configured gives `None`.
    async fn open(variable: &str, tag: &str) -> Option<(Scratch, Box<dyn DatabaseDriver>, Server)> {
        let server = live::server(variable)?;
        let mut admin = server.open(DbType::Mysql, None).await;
        let name = live::unique_name(tag);
        live::run(admin.as_mut(), &format!("CREATE DATABASE {name}")).await;
        let mut driver = server.open(DbType::Mysql, Some(&name)).await;
        let scratch = Scratch { admin, name };
        if let Err(error) = live::load_fixture(driver.as_mut(), FIXTURE).await {
            scratch.remove().await;
            panic!("the fixture failed: {error}");
        }
        Some((scratch, driver, server))
    }

    async fn remove(mut self) {
        let sql = format!("DROP DATABASE IF EXISTS {}", self.name);
        live::run(self.admin.as_mut(), &sql).await;
    }
}

fn trigger(name: &str, timing: TriggerTiming, event: TriggerEvent) -> Trigger {
    Trigger {
        name: name.to_string(),
        timing,
        events: vec![event],
        enabled: true,
        replica: false,
        update_columns: Vec::new(),
    }
}

fn event(name: &str, enabled: bool, schedule: &str) -> ScheduledEvent {
    ScheduledEvent {
        name: name.to_string(),
        enabled,
        schedule: Some(schedule.to_string()),
    }
}

/// Reads the CREATE text of an object of the database.
async fn object_text(
    driver: &mut dyn DatabaseDriver,
    database: &str,
    name: &str,
    object_type: ObjectType,
) -> String {
    let query = driver.object_create_query(Some(database), None, None, name, object_type);
    live::create_text(driver, query).await
}

/// Lists the triggers and the events, and the relations of the snapshot.
async fn triggers_and_events(variable: &str, tag: &str) {
    use TriggerEvent::{Delete, Insert, Update};
    use TriggerTiming::{After, Before};
    let Some((scratch, mut driver, _)) = Scratch::open(variable, tag).await else {
        return;
    };
    let database = scratch.name.clone();
    let body = async move {
        let driver = driver.as_mut();
        assert_eq!(
            driver
                .list_triggers(&database, None, "orders")
                .await
                .unwrap(),
            [
                trigger("bi_stamp", Before, Insert),
                trigger("bi_orders", Before, Insert),
                trigger("bi_check", Before, Insert),
                trigger("bu_orders", Before, Update),
                trigger("bd_orders", Before, Delete),
                trigger("au_orders", After, Update),
            ]
        );
        assert_eq!(
            driver.list_events(&database, None).await.unwrap(),
            [
                event("ev_body", true, "EVERY 1 HOUR"),
                event("ev_daily", true, "EVERY 1 DAY"),
                event("ev_mark", true, "EVERY 1 WEEK"),
                event("ev_off", false, "EVERY 5 MINUTE"),
                event("ev_once", true, "AT 2030-06-01 12:34:56"),
            ]
        );

        let tables = driver.list_tables(&database, None).await.unwrap();
        assert_eq!(tables, [Table::table("orders"), Table::view("big_orders")]);
        let snapshot = driver.schema_snapshot(&database, 10_000).await.unwrap();
        assert!(snapshot.complete);
        assert_eq!(snapshot.relations.len(), 2);
        assert_eq!(snapshot.column_count, 5);

        // A bound under the count of columns keeps that many columns and
        // marks the snapshot as not complete. A bound of the exact count
        // gives the whole snapshot.
        let snapshot = driver.schema_snapshot(&database, 3).await.unwrap();
        assert!(!snapshot.complete);
        assert_eq!(snapshot.column_count, 3);
        let snapshot = driver.schema_snapshot(&database, 5).await.unwrap();
        assert!(snapshot.complete);
        assert_eq!(snapshot.column_count, 5);
    };
    live::with_cleanup(body, scratch.remove()).await;
}

/// Runs the CREATE text of a view, a trigger and an event again after a
/// drop.
async fn scripts_round_trip(variable: &str, tag: &str) {
    let Some((scratch, mut driver, _)) = Scratch::open(variable, tag).await else {
        return;
    };
    let database = scratch.name.clone();
    let body = async move {
        let driver = driver.as_mut();
        // A body of more than one statement comes between DELIMITER
        // commands, and a body of one statement stays bare. The body of
        // ev_mark contains $$, so its terminator is $$$.
        for (name, object_type, word, delimiter) in [
            ("bi_orders", ObjectType::Trigger, "TRIGGER", None),
            ("au_orders", ObjectType::Trigger, "TRIGGER", None),
            ("bu_orders", ObjectType::Trigger, "TRIGGER", Some("$$")),
            ("ev_daily", ObjectType::Event, "EVENT", None),
            ("ev_once", ObjectType::Event, "EVENT", None),
            ("ev_body", ObjectType::Event, "EVENT", Some("$$")),
            ("ev_mark", ObjectType::Event, "EVENT", Some("$$$")),
        ] {
            let text = object_text(driver, &database, name, object_type).await;
            assert_eq!(
                text.starts_with("DELIMITER "),
                delimiter.is_some(),
                "{text}"
            );
            if let Some(delimiter) = delimiter {
                let start = format!("DELIMITER {delimiter}\n");
                let end = format!("END{delimiter}\nDELIMITER ;");
                assert!(text.starts_with(&start) && text.ends_with(&end), "{text}");
            }
            live::run(driver, &format!("DROP {word} {name}")).await;
            live::run(driver, &text).await;
            assert_eq!(
                object_text(driver, &database, name, object_type).await,
                text
            );
        }

        // The CREATE text has no FOLLOWS clause, so bi_orders, made again
        // from its text, fires last of the BEFORE INSERT triggers.
        let names: Vec<String> = driver
            .list_triggers(&database, None, "orders")
            .await
            .unwrap()
            .into_iter()
            .map(|trigger| trigger.name)
            .collect();
        assert_eq!(names[..3], ["bi_stamp", "bi_check", "bi_orders"]);

        let read = |driver: &mut dyn DatabaseDriver| {
            driver.create_query(Some(&database), None, "big_orders", RelationType::View)
        };
        let query = read(driver);
        let text = live::create_text(driver, query).await;
        live::run(driver, "DROP VIEW big_orders").await;
        live::run(driver, &text).await;
        let query = read(driver);
        assert_eq!(live::create_text(driver, query).await, text);
    };
    live::with_cleanup(body, scratch.remove()).await;
}

#[tokio::test]
#[ignore = "needs a live MySQL server"]
async fn live_mysql_lists_the_triggers_and_the_events() {
    triggers_and_events("SQLX_LIVE_MYSQL", "my_list").await;
}

#[tokio::test]
#[ignore = "needs a live MariaDB server"]
async fn live_mariadb_lists_the_triggers_and_the_events() {
    triggers_and_events("SQLX_LIVE_MARIADB", "maria_list").await;
}

#[tokio::test]
#[ignore = "needs a live MySQL server"]
async fn live_mysql_create_text_runs_again_after_a_drop() {
    scripts_round_trip("SQLX_LIVE_MYSQL", "my_scripts").await;
}

#[tokio::test]
#[ignore = "needs a live MariaDB server"]
async fn live_mariadb_create_text_runs_again_after_a_drop() {
    scripts_round_trip("SQLX_LIVE_MARIADB", "maria_scripts").await;
}

/// Reads the generated columns and the place of a syntax error on one
/// server.
async fn generated_columns_and_error_lines(variable: &str, tag: &str) {
    let Some((scratch, mut driver, _server)) = Scratch::open(variable, tag).await else {
        return;
    };
    live::run(
        driver.as_mut(),
        "CREATE TABLE gen (id INT AUTO_INCREMENT PRIMARY KEY, a INT, \
         b INT AS (a * 2) VIRTUAL, c INT AS (a + 1) STORED, \
         d DATETIME DEFAULT CURRENT_TIMESTAMP)",
    )
    .await;
    let columns = driver
        .list_columns(&scratch.name, None, "gen")
        .await
        .unwrap();
    let generated: Vec<bool> = columns.iter().map(|column| column.is_generated).collect();
    assert_eq!(generated, vec![true, false, true, true, false]);

    let error = driver
        .execute_query(
            "SELECT 1;\nSELECT\n  1 FROM FROM",
            None,
            &crate::db::ExecOptions::default(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, crate::error::Error::Located { line: 3, .. }),
        "{error:?}"
    );

    scratch.remove().await;
}

#[tokio::test]
#[ignore = "needs a live MySQL server"]
async fn live_mysql_marks_generated_columns_and_error_lines() {
    generated_columns_and_error_lines("SQLX_LIVE_MYSQL", "my_gen").await;
}

#[tokio::test]
#[ignore = "needs a live MariaDB server"]
async fn live_mariadb_marks_generated_columns_and_error_lines() {
    generated_columns_and_error_lines("SQLX_LIVE_MARIADB", "maria_gen").await;
}

/// A read of 1,000,000 numbers in order, from 1. The rows fill the buffers
/// of the connection, so the server must wait while the read is paused.
fn numbers_query() -> String {
    numbers_up_to(6)
}

/// A read of the numbers from 1 to 10 to the power of `places`, in order.
fn numbers_up_to(places: u32) -> String {
    let digits = "(SELECT 0 AS d UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 \
                  UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 \
                  UNION ALL SELECT 8 UNION ALL SELECT 9)";
    let tables: Vec<String> = (0..places).map(|n| format!("{digits} AS t{n}")).collect();
    let sum: Vec<String> = (0..places)
        .map(|n| format!("t{n}.d * {}", 10_i64.pow(n)))
        .collect();
    format!(
        "SELECT {} + 1 AS n FROM {} ORDER BY n",
        sum.join(" + "),
        tables.join(" CROSS JOIN ")
    )
}

/// Pauses a read on a session whose `net_write_timeout` is two seconds,
/// waits past that time, and exports every row. The session then has its
/// own value again.
async fn a_paused_read_exports_every_row_once(variable: &str) {
    let Some(server) = live::server(variable) else {
        return;
    };
    let mut driver = server.open(DbType::Mysql, None).await;
    live::run(driver.as_mut(), "SET SESSION net_write_timeout = 2").await;
    let paused = live::pause_read(driver, &numbers_query(), 100, Duration::from_secs(60)).await;
    assert_eq!(live::numbers(&paused.grid), (1..=100).collect::<Vec<_>>());
    tokio::time::sleep(Duration::from_secs(5)).await;
    let rows = live::export_paused(&paused.read, usize::MAX).await.unwrap();
    assert_eq!(live::numbers(&rows), (1..=1_000_000).collect::<Vec<_>>());
    let response =
        live::run_after(&paused.session, "SELECT @@SESSION.net_write_timeout AS t").await;
    assert_eq!(live::cell(&response, 0, 0).as_deref(), Some("2"));
}

/// Releases a paused read, and runs a second statement on its session.
async fn a_released_read_frees_its_session(variable: &str) {
    let Some(server) = live::server(variable) else {
        return;
    };
    let driver = server.open(DbType::Mysql, None).await;
    let paused = live::pause_read(driver, &numbers_query(), 100, Duration::from_secs(60)).await;
    let session = paused.session.clone();
    drop(paused);
    let response = live::run_after(&session, "SELECT 2 AS two").await;
    assert_eq!(live::cell(&response, 0, 0).as_deref(), Some("2"));
}

/// Lets the limit of the pause end a read.
async fn the_end_of_the_pause_releases_the_read(variable: &str) {
    let Some(server) = live::server(variable) else {
        return;
    };
    let driver = server.open(DbType::Mysql, None).await;
    let paused = live::pause_read(driver, &numbers_query(), 100, Duration::from_secs(1)).await;
    let response = live::run_after(&paused.session, "SELECT 3 AS three").await;
    assert_eq!(live::cell(&response, 0, 0).as_deref(), Some("3"));
    assert!(live::export_paused(&paused.read, usize::MAX).await.is_err());
}

#[tokio::test]
#[ignore = "needs a live MySQL server"]
async fn live_mysql_a_paused_read_exports_every_row_once() {
    a_paused_read_exports_every_row_once("SQLX_LIVE_MYSQL").await;
}

#[tokio::test]
#[ignore = "needs a live MariaDB server"]
async fn live_mariadb_a_paused_read_exports_every_row_once() {
    a_paused_read_exports_every_row_once("SQLX_LIVE_MARIADB").await;
}

#[tokio::test]
#[ignore = "needs a live MySQL server"]
async fn live_mysql_a_released_read_frees_its_session() {
    a_released_read_frees_its_session("SQLX_LIVE_MYSQL").await;
}

#[tokio::test]
#[ignore = "needs a live MariaDB server"]
async fn live_mariadb_a_released_read_frees_its_session() {
    a_released_read_frees_its_session("SQLX_LIVE_MARIADB").await;
}

#[tokio::test]
#[ignore = "needs a live MySQL server"]
async fn live_mysql_the_end_of_the_pause_releases_the_read() {
    the_end_of_the_pause_releases_the_read("SQLX_LIVE_MYSQL").await;
}

#[tokio::test]
#[ignore = "needs a live MariaDB server"]
async fn live_mariadb_the_end_of_the_pause_releases_the_read() {
    the_end_of_the_pause_releases_the_read("SQLX_LIVE_MARIADB").await;
}

const LOCK_PROBE: &str = "CREATE TABLE lock_probe (id int PRIMARY KEY, v int) ENGINE = InnoDB;\n\
                          INSERT INTO lock_probe VALUES (1, 1);";

/// Makes a lock in the scratch database and gives the report that a third
/// session read while a second session waited for it.
async fn locked_report(
    server: &Server,
    name: &str,
    reporter: &mut dyn DatabaseDriver,
    lock: &str,
    wait: &str,
) -> live::LockedReport {
    let holder = server.open(DbType::Mysql, Some(name)).await;
    let waiter = server.open(DbType::Mysql, Some(name)).await;
    let scene = live::LockScene {
        session_id: "SELECT CONNECTION_ID()",
        lock,
        wait,
        release: "ROLLBACK",
    };
    live::report_during_wait(holder, waiter, reporter, &scene).await
}

/// Finds the wait of the waiting session of a scene, and checks that the
/// blocking session of the scene causes it.
fn wait_of(locked: &live::LockedReport) -> &crate::db::blocking::BlockingSession {
    let report = &locked.report;
    let wait = report
        .sessions
        .iter()
        .find(|row| row.waiting_session == locked.waiter)
        .unwrap_or_else(|| panic!("no wait in {report:?}"));
    assert_eq!(wait.blocking_session, locked.holder, "{report:?}");
    wait
}

/// Checks the report of a row lock, and of a metadata lock where the server
/// records them.
async fn the_report_names_the_session_that_blocks(variable: &str, tag: &str, metadata: bool) {
    let Some((scratch, mut driver, server)) = Scratch::open(variable, tag).await else {
        return;
    };
    let name = scratch.name.clone();
    let body = async move {
        live::run(driver.as_mut(), LOCK_PROBE).await;

        let locked = locked_report(
            &server,
            &name,
            driver.as_mut(),
            "START TRANSACTION; UPDATE lock_probe SET v = 2 WHERE id = 1;",
            "UPDATE lock_probe SET v = 3 WHERE id = 1",
        )
        .await;
        let wait = wait_of(&locked);
        assert!(
            wait.object
                .as_deref()
                .is_some_and(|object| object.contains("lock_probe")),
            "{wait:?}"
        );
        assert!(wait
            .lock_mode
            .as_deref()
            .is_some_and(|mode| mode.starts_with('X')));
        assert_eq!(wait.blocking_status.as_deref(), Some("Sleep"));
        assert_eq!(
            wait.waiting_statement.as_deref(),
            Some("UPDATE lock_probe SET v = 3 WHERE id = 1")
        );
        assert!(wait.wait_ms.is_some());
        assert!(locked
            .report
            .open_transactions
            .iter()
            .any(|row| row.session == locked.holder));

        let locked = locked_report(
            &server,
            &name,
            driver.as_mut(),
            "START TRANSACTION; SELECT v FROM lock_probe;",
            "ALTER TABLE lock_probe ADD COLUMN w int",
        )
        .await;
        if metadata {
            let wait = wait_of(&locked);
            assert_eq!(wait.object, Some(format!("{name}.lock_probe")));
            assert!(wait
                .blocking_statement
                .as_deref()
                .is_some_and(|text| text.contains("SELECT v FROM lock_probe")));
            assert!(locked.report.notes.is_empty(), "{:?}", locked.report);
        } else {
            assert!(
                locked
                    .report
                    .notes
                    .iter()
                    .any(|note| note.contains("performance_schema")),
                "{:?}",
                locked.report
            );
        }
    };
    live::with_cleanup(body, scratch.remove()).await;
}

/// Checks that a user without the PROCESS privilege gets notes in place of
/// an error.
async fn a_user_without_process_gets_notes(variable: &str, tag: &str) {
    let Some((scratch, mut driver, server)) = Scratch::open(variable, tag).await else {
        return;
    };
    let name = scratch.name.clone();
    let user = live::unique_name("noproc");
    let password = "LiveNoProcess2026";
    live::run(
        driver.as_mut(),
        &format!(
            "CREATE USER '{user}'@'%' IDENTIFIED BY '{password}';\n\
             GRANT ALL ON {name}.* TO '{user}'@'%';"
        ),
    )
    .await;
    let limited = Server {
        user: user.clone(),
        password: password.to_string(),
        ..server.clone()
    };
    let body = async move {
        let mut reporter = limited.open(DbType::Mysql, Some(&name)).await;
        let report = reporter.blocking_sessions().await.unwrap();
        assert!(report.sessions.is_empty());
        assert!(
            report
                .notes
                .iter()
                .any(|note| note.contains("aren't shown")),
            "{report:?}"
        );
    };
    let cleanup = async move {
        live::run(driver.as_mut(), &format!("DROP USER '{user}'@'%'")).await;
        scratch.remove().await;
    };
    live::with_cleanup(body, cleanup).await;
}

#[tokio::test]
#[ignore = "needs a live MySQL server"]
async fn live_mysql_the_report_names_the_session_that_blocks() {
    the_report_names_the_session_that_blocks("SQLX_LIVE_MYSQL", "my_block", true).await;
}

#[tokio::test]
#[ignore = "needs a live MariaDB server"]
async fn live_mariadb_the_report_names_the_session_that_blocks() {
    the_report_names_the_session_that_blocks("SQLX_LIVE_MARIADB", "maria_block", false).await;
}

#[tokio::test]
#[ignore = "needs a live MySQL server"]
async fn live_mysql_a_user_without_process_gets_notes() {
    a_user_without_process_gets_notes("SQLX_LIVE_MYSQL", "my_noproc").await;
}

#[tokio::test]
#[ignore = "needs a live MariaDB server"]
async fn live_mariadb_a_user_without_process_gets_notes() {
    a_user_without_process_gets_notes("SQLX_LIVE_MARIADB", "maria_noproc").await;
}

/// Runs a procedure that returns more rows than the limit. A procedure can
/// write, so the driver reads every row that remains and tells the sink.
async fn a_drained_procedure_tells_the_sink(variable: &str, tag: &str) {
    let Some((scratch, mut driver, _)) = Scratch::open(variable, tag).await else {
        return;
    };
    let body = async {
        let create = format!("CREATE PROCEDURE many_numbers() {}", numbers_up_to(3));
        live::run(driver.as_mut(), &create).await;
        let options = crate::db::ExecOptions {
            max_rows: 10,
            ..crate::db::ExecOptions::default()
        };
        let mut sink = crate::db::sink::probe::Telling::new(10);
        driver
            .execute_stream("CALL many_numbers()", None, &options, &mut sink)
            .await
            .unwrap();
        assert_eq!(sink.told, 1);
        let response = sink
            .buffer
            .into_response(crate::db::sink::RunSummary::default());
        assert_eq!(response.results[0].rows.len(), 10);
        assert!(response.results[0].truncated);
    };
    live::with_cleanup(body, scratch.remove()).await;
}

#[tokio::test]
#[ignore = "needs a live MySQL server"]
async fn live_mysql_a_drained_procedure_tells_the_sink() {
    a_drained_procedure_tells_the_sink("SQLX_LIVE_MYSQL", "my_drain").await;
}

#[tokio::test]
#[ignore = "needs a live MariaDB server"]
async fn live_mariadb_a_drained_procedure_tells_the_sink() {
    a_drained_procedure_tells_the_sink("SQLX_LIVE_MARIADB", "maria_drain").await;
}

#[tokio::test]
#[ignore = "needs a live MySQL server"]
async fn live_mysql_a_paused_read_names_its_session_and_the_sessions_it_blocks() {
    let Some((scratch, mut driver, server)) = Scratch::open("SQLX_LIVE_MYSQL", "my_paused").await
    else {
        return;
    };
    let name = scratch.name.clone();
    let body = async move {
        live::run(driver.as_mut(), LOCK_PROBE).await;
        let reader = server.open(DbType::Mysql, Some(&name)).await;
        let waiter = server.open(DbType::Mysql, Some(&name)).await;
        // The statement that waits for the client keeps a metadata lock of
        // the table, and ALTER TABLE waits for that lock.
        let read = numbers_query().replace(" FROM ", " FROM lock_probe CROSS JOIN ");
        let scene = live::PausedLockScene {
            session_id: "SELECT CONNECTION_ID()",
            read: &read,
            wait: "ALTER TABLE lock_probe ADD COLUMN w int",
        };
        let locked = live::report_on_paused_read(reader, waiter, driver.as_mut(), &scene).await;
        live::assert_paused_read_blocks(&locked);
    };
    live::with_cleanup(body, scratch.remove()).await;
}
