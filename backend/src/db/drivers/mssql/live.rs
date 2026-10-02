//! The tests of the MS SQL Server driver against a live server. The variable
//! `SQLX_LIVE_MSSQL` names the server, see [`crate::db::drivers::live`].

use crate::db::drivers::live::{self, Server};
use crate::db::drivers::DatabaseDriver;
use crate::db::{ObjectType, RelationType, Table, Trigger, TriggerEvent, TriggerTiming};
use crate::storage::DbType;

const FIXTURE: &str = include_str!("../../../../live/fixtures/mssql.sql");

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

        let snapshot = driver.schema_snapshot(&database, 10_000).await.unwrap();
        assert!(snapshot.complete);
        assert!(snapshot
            .relations
            .iter()
            .any(|relation| relation.name == "Orders" && relation.columns.len() == 6));
    };
    live::with_cleanup(body, scratch.remove()).await;
}
