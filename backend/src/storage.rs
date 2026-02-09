use anyhow::Context;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{BufReader, BufWriter},
    path::PathBuf,
};

use crate::error::Result;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum AuthType {
    Sql,
    Integrated,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SavedConnection {
    pub name: String,
    pub server: String,
    pub database: String,
    pub auth_type: AuthType,
    pub user: Option<String>,
}

fn get_project_dirs() -> Result<ProjectDirs> {
    Ok(ProjectDirs::from("com", "gemini", "SQL Explorer")
        .context("Failed to get project directories")?)
}

fn get_connections_file_path() -> Result<PathBuf> {
    let proj_dirs = get_project_dirs()?;
    let data_dir = proj_dirs.data_dir();
    if !data_dir.exists() {
        fs::create_dir_all(data_dir)
            .with_context(|| format!("Failed to create data directory at {:?}", data_dir))?;
    }
    Ok(data_dir.join("connections.json"))
}

pub fn get_all_connections() -> Result<Vec<SavedConnection>> {
    let path = get_connections_file_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path).context("Failed to open connections file")?;
    let reader = BufReader::new(file);
    let connections: Vec<SavedConnection> =
        serde_json::from_reader(reader).context("Failed to deserialize connections")?;
    Ok(connections)
}

pub fn save_connection_details(connection: &SavedConnection) -> Result<()> {
    let mut connections = get_all_connections()?;
    // If a connection with the same name exists, remove it.
    connections.retain(|c| c.name != connection.name);
    connections.push(connection.clone());

    let path = get_connections_file_path()?;
    let file = File::create(path).context("Failed to create connections file")?;
    let writer = BufWriter::new(file);
    serde_json::to_writer_pretty(writer, &connections)
        .context("Failed to serialize and save connections")?;
    Ok(())
}

pub fn delete_connection_details(connection_name: &str) -> Result<()> {
    let mut connections = get_all_connections()?;
    connections.retain(|c| c.name != connection_name);

    let path = get_connections_file_path()?;
    let file = File::create(path).context("Failed to create connections file")?;
    let writer = BufWriter::new(file);
    serde_json::to_writer_pretty(writer, &connections)
        .context("Failed to serialize and save connections")?;
    Ok(())
}

const KEYRING_SERVICE: &str = "SQL Explorer";

pub fn save_password(connection_name: &str, password: &str) -> Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, connection_name)?;
    entry
        .set_password(password)
        .context("Failed to save password to keyring")?;
    Ok(())
}

pub fn get_password(connection_name: &str) -> Result<String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, connection_name)?;
    Ok(entry
        .get_password()
        .context("Failed to get password from keyring")?)
}

pub fn delete_password(connection_name: &str) -> Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, connection_name)?;
    match entry.delete_password() {
        Ok(_) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()), // It's ok if there's no entry to delete
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn cleanup(connection_name: &str) {
        if let Ok(path) = get_connections_file_path() {
            if path.exists() {
                let _ = fs::remove_file(path);
            }
        }
        let _ = delete_password(connection_name);
    }

    #[test]
    #[ignore]
    fn test_save_and_get_connections() {
        let conn_name = "test_conn_1";
        let conn = SavedConnection {
            name: conn_name.to_string(),
            server: "localhost".to_string(),
            database: "master".to_string(),
            auth_type: AuthType::Sql,
            user: Some("sa".to_string()),
        };

        // Cleanup before test
        cleanup(conn_name);
        cleanup("test_conn_2");

        // Save
        let save_result = save_connection_details(&conn);
        assert!(save_result.is_ok());

        // Get all
        let connections_result = get_all_connections();
        assert!(connections_result.is_ok());
        let connections = connections_result.unwrap();
        assert_eq!(connections.len(), 1);
        assert_eq!(connections[0].name, conn_name);
        assert_eq!(connections[0].auth_type, AuthType::Sql);

        // Cleanup after test
        cleanup(conn_name);
    }

    #[test]
    #[ignore]
    fn test_delete_connection() {
        let conn_name = "test_conn_2";
        let conn = SavedConnection {
            name: conn_name.to_string(),
            server: "localhost".to_string(),
            database: "master".to_string(),
            auth_type: AuthType::Integrated,
            user: None,
        };

        // Cleanup before test
        cleanup(conn_name);

        // Save
        assert!(save_connection_details(&conn).is_ok());

        // Check if saved
        let connections = get_all_connections().unwrap();
        assert_eq!(connections.len(), 1);

        // Delete
        assert!(delete_connection_details(conn_name).is_ok());

        // Check if deleted
        let connections_after_delete = get_all_connections().unwrap();
        assert!(connections_after_delete.is_empty());

        // Cleanup after test
        cleanup(conn_name);
    }

    #[test]
    #[ignore]
    fn test_password_management() {
        let conn_name = "test_password_conn";
        let password = "my_secret_password";

        // Cleanup before test
        cleanup(conn_name);

        // Save password
        assert!(save_password(conn_name, password).is_ok());

        // Get password
        let retrieved_password_result = get_password(conn_name);
        assert!(retrieved_password_result.is_ok());
        assert_eq!(retrieved_password_result.unwrap(), password);

        // Delete password
        assert!(delete_password(conn_name).is_ok());

        // Try to get deleted password
        let retrieved_password_after_delete = get_password(conn_name);
        assert!(retrieved_password_after_delete.is_err());

        // Cleanup after test
        cleanup(conn_name);
    }
}