use crate::config::{self, Config, Machine};
use anyhow::{Context, Result};
use std::{
    collections::HashSet,
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
use toml_edit::{ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value, value};

/// Merge discovered identity only; leave repo associations and user settings alone.
pub fn lima(path: &Path, machines: &[String], dry_run: bool) -> Result<Vec<String>> {
    let path = config::expand(path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(!dry_run)
        .open(&path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    // Serialize imports and re-read after locking so a second import sees the first.
    file.lock()
        .with_context(|| format!("cannot lock {}", path.display()))?;
    let mut source = String::new();
    file.read_to_string(&mut source)?;
    let config = Config::parse(&source)?;
    let mut names: HashSet<_> = config.workspace.iter().map(|ws| ws.name.clone()).collect();
    let mut registered: HashSet<_> = config
        .workspace
        .iter()
        .filter_map(|ws| match &ws.machine {
            Machine::Lima { name } => Some(name.clone()),
            _ => None,
        })
        .collect();
    let mut document: DocumentMut = source.parse()?;
    let mut added = Vec::new();
    for machine in machines {
        if !registered.insert(machine.clone()) {
            continue;
        }
        let mut name = machine.clone();
        if names.contains(&name) {
            name = format!("lima-{machine}");
            let base = name.clone();
            let mut suffix = 2;
            while names.contains(&name) {
                name = format!("{base}-{suffix}");
                suffix += 1;
            }
        }
        names.insert(name.clone());
        let mut identity = InlineTable::new();
        identity.insert("backend", "lima".into());
        identity.insert("name", machine.as_str().into());
        let mut workspace = Table::new();
        workspace.insert("name", value(&name));
        workspace.insert("machine", value(identity));
        let workspaces = document
            .entry("workspace")
            .or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
        if let Some(array) = workspaces.as_array_of_tables_mut() {
            array.push(workspace);
        } else {
            // Also accept hand-written `workspace = []` / arrays of inline tables.
            workspaces
                .as_array_mut()
                .context("workspace must be an array")?
                .push(Value::InlineTable(workspace.into_inline_table()));
        }
        added.push(name);
    }
    if !added.is_empty() {
        let updated = document.to_string();
        Config::parse(&updated).context("import would produce invalid config")?;
        if !dry_run {
            file.seek(SeekFrom::Start(0))?;
            file.write_all(updated.as_bytes())
                .with_context(|| format!("cannot write {}", path.display()))?;
            file.set_len(updated.len() as u64)?;
            file.sync_all()?;
        }
    }
    Ok(added)
}
