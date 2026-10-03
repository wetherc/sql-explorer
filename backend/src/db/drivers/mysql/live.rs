//! The tests of the MySQL driver against a live MySQL server and a live
//! MariaDB server. The variables `SQLX_LIVE_MYSQL` and `SQLX_LIVE_MARIADB`
//! name the servers, see [`crate::db::drivers::live`].

use crate::db::drivers::live::{self, Server};
use crate::db::drivers::DatabaseDriver;
use crate::db::{
    ObjectType, RelationType, ScheduledEvent, Table, Trigger, TriggerEvent, TriggerTiming,
};
use crate::storage::DbType;

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
