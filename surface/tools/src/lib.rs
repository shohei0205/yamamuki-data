use anyhow::{Result, ensure};
use std::path::Path;
use tempfile::NamedTempFile;
use yamamuki_data_sqlite::{self as db, rusqlite::Connection};

pub fn write_database(
    output: &Path,
    container: bool,
    write: impl FnOnce(&mut Connection) -> Result<()>,
) -> Result<()> {
    ensure!(
        !output.exists(),
        "出力先がすでに存在します: {}",
        output.display()
    );
    let directory = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let file = NamedTempFile::new_in(directory)?.into_temp_path();
    let mut connection = Connection::open(&file)?;
    if container {
        db::initialize(&connection)?;
    }
    write(&mut connection)?;
    if container {
        db::validate(&connection)?;
    }
    connection.close().map_err(|(_, error)| error)?;
    file.persist_noclobber(output)?;
    Ok(())
}
