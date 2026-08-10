//! Builds the statements that the explorer offers for one relation. The
//! backend builds them, so that every name is quoted for the engine and the
//! user interface holds no rule of a dialect.

use serde::{Deserialize, Serialize};

use crate::db::AppColumn;
use crate::sql::Dialect;

/// The statement that the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScriptKind {
    Create,
    Select,
    Insert,
    Update,
}

/// The text that stands in the place of a value the user must give.
const VALUE_MARK: &str = "NULL";

/// Builds `SELECT`, with one column on each line. A relation that reports no
/// column gets an asterisk, because such a statement still runs.
pub fn select_statement(dialect: Dialect, name: &str, columns: &[AppColumn]) -> String {
    if columns.is_empty() {
        return format!("SELECT *\nFROM {name};");
    }
    let list = columns
        .iter()
        .map(|column| format!("    {}", dialect.quote_identifier(&column.name)))
        .collect::<Vec<_>>()
        .join(",\n");
    format!("SELECT\n{list}\nFROM {name};")
}

/// Builds `INSERT`, with one column on each line and one value for each
/// column. Each value is a mark that the user replaces, and the comment
/// beside it names the column and its type.
pub fn insert_statement(dialect: Dialect, name: &str, columns: &[AppColumn]) -> String {
    let names = columns
        .iter()
        .map(|column| format!("    {}", dialect.quote_identifier(&column.name)))
        .collect::<Vec<_>>()
        .join(",\n");
    let values = join_lines(columns, |column, last| {
        format!(
            "    {VALUE_MARK}{} -- {}: {}",
            if last { "" } else { "," },
            column.name,
            column.data_type
        )
    });
    format!("INSERT INTO {name} (\n{names}\n)\nVALUES (\n{values}\n);")
}

/// Builds `UPDATE`. The columns of the primary key stand in the WHERE clause
/// and the other columns stand in the SET clause. A relation whose every
/// column belongs to the key gets every column in the SET clause as well,
/// because a SET clause cannot be empty.
///
/// A relation with no primary key gets a WHERE clause that matches no row,
/// so a statement that ran by mistake changes nothing.
pub fn update_statement(dialect: Dialect, name: &str, columns: &[AppColumn]) -> String {
    let keys: Vec<&AppColumn> = columns
        .iter()
        .filter(|column| column.is_primary_key)
        .collect();
    let rest: Vec<AppColumn> = columns
        .iter()
        .filter(|column| !column.is_primary_key)
        .cloned()
        .collect();
    let assigned = if rest.is_empty() { columns } else { &rest };

    let sets = join_lines(assigned, |column, last| {
        format!(
            "    {} = {VALUE_MARK}{} -- {}",
            dialect.quote_identifier(&column.name),
            if last { "" } else { "," },
            column.data_type
        )
    });

    let where_clause = if keys.is_empty() {
        "    1 = 0 -- No primary key was found. Name the rows to change.".to_string()
    } else {
        keys.iter()
            .enumerate()
            .map(|(index, column)| {
                let lead = if index == 0 { "   " } else { "    AND" };
                format!(
                    "{lead} {} = {VALUE_MARK} -- {}",
                    dialect.quote_identifier(&column.name),
                    column.data_type
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!("UPDATE {name}\nSET\n{sets}\nWHERE\n{where_clause};")
}

/// Builds a draft of `CREATE TABLE` from the column list. This is the
/// fallback for an engine that gives no text of its own.
///
/// The draft holds the columns, their types, the null rule and the primary
/// key. It holds no index, no default, no other constraint and no collation,
/// so the header states that the text is a draft.
pub fn create_draft(dialect: Dialect, name: &str, columns: &[AppColumn]) -> String {
    let mut lines: Vec<String> = columns
        .iter()
        .map(|column| {
            format!(
                "    {} {} {}",
                dialect.quote_identifier(&column.name),
                column.data_type,
                if column.nullable { "NULL" } else { "NOT NULL" }
            )
        })
        .collect();

    let keys: Vec<String> = columns
        .iter()
        .filter(|column| column.is_primary_key)
        .map(|column| dialect.quote_identifier(&column.name))
        .collect();
    if !keys.is_empty() {
        lines.push(format!("    PRIMARY KEY ({})", keys.join(", ")));
    }

    format!(
        "-- This text is a draft, built from the column list.\n\
         -- It holds no index, no default, no other constraint and no collation.\n\
         CREATE TABLE {name} (\n{}\n);",
        lines.join(",\n")
    )
}

/// Writes one line for each column and marks the last line, so that the
/// comment of a line stands after the comma and not in front of it.
fn join_lines<F>(columns: &[AppColumn], line: F) -> String
where
    F: Fn(&AppColumn, bool) -> String,
{
    let last = columns.len().saturating_sub(1);
    columns
        .iter()
        .enumerate()
        .map(|(index, column)| line(column, index == last))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str, data_type: &str, nullable: bool, key: bool) -> AppColumn {
        AppColumn {
            name: name.to_string(),
            data_type: data_type.to_string(),
            nullable,
            is_primary_key: key,
        }
    }

    fn two_columns() -> Vec<AppColumn> {
        vec![
            column("id", "int", false, true),
            column("name", "nvarchar(50)", true, false),
        ]
    }

    #[test]
    fn the_kind_round_trips_through_json() {
        let text = serde_json::to_string(&ScriptKind::Create).unwrap();
        assert_eq!(text, "\"create\"");
        assert_eq!(
            serde_json::from_str::<ScriptKind>("\"update\"").unwrap(),
            ScriptKind::Update
        );
    }

    #[test]
    fn a_select_names_each_column() {
        let text = select_statement(Dialect::MsSql, "[dbo].[t]", &two_columns());
        assert_eq!(text, "SELECT\n    [id],\n    [name]\nFROM [dbo].[t];");
    }

    #[test]
    fn a_select_without_columns_reads_every_column() {
        assert_eq!(
            select_statement(Dialect::Postgres, "\"t\"", &[]),
            "SELECT *\nFROM \"t\";"
        );
    }

    #[test]
    fn an_insert_gives_one_value_for_each_column() {
        let text = insert_statement(Dialect::MySql, "`db`.`t`", &two_columns());
        assert_eq!(
            text,
            "INSERT INTO `db`.`t` (\n    `id`,\n    `name`\n)\nVALUES (\n    \
             NULL, -- id: int\n    NULL -- name: nvarchar(50)\n);"
        );
    }

    #[test]
    fn an_update_puts_the_key_in_the_where_clause() {
        let mut columns = two_columns();
        columns.push(column("code", "int", false, true));
        let text = update_statement(Dialect::Postgres, "\"t\"", &columns);
        assert_eq!(
            text,
            "UPDATE \"t\"\nSET\n    \"name\" = NULL -- nvarchar(50)\nWHERE\n    \
             \"id\" = NULL -- int\n    AND \"code\" = NULL -- int;"
        );
    }

    #[test]
    fn an_update_without_a_key_matches_no_row() {
        let columns = vec![column("name", "text", true, false)];
        let text = update_statement(Dialect::Sqlite, "\"t\"", &columns);
        assert!(text.contains("WHERE\n    1 = 0 -- No primary key was found."));
    }

    #[test]
    fn an_update_of_key_columns_alone_sets_every_column() {
        let columns = vec![column("id", "int", false, true)];
        let text = update_statement(Dialect::Sqlite, "\"t\"", &columns);
        assert!(text.contains("SET\n    \"id\" = NULL -- int"));
        assert!(text.contains("WHERE\n    \"id\" = NULL -- int"));
    }

    #[test]
    fn a_draft_holds_the_types_the_null_rule_and_the_key() {
        let text = create_draft(Dialect::MsSql, "[dbo].[t]", &two_columns());
        assert!(text.starts_with("-- This text is a draft, built from the column list."));
        assert!(text.contains("CREATE TABLE [dbo].[t] (\n    [id] int NOT NULL,"));
        assert!(text.contains("    [name] nvarchar(50) NULL,"));
        assert!(text.ends_with("    PRIMARY KEY ([id])\n);"));
    }

    #[test]
    fn a_draft_without_a_key_holds_the_columns_alone() {
        let columns = vec![column("name", "text", true, false)];
        let text = create_draft(Dialect::Sqlite, "\"t\"", &columns);
        assert!(text.ends_with("CREATE TABLE \"t\" (\n    \"name\" text NULL\n);"));
    }
}
