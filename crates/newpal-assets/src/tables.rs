//! Value tables with mod overrides (JSON or TOML).
//!
//! Override file `tables/<name>.json` or `tables/<name>.toml` in a mod:
//! ```toml
//! [[records]]
//! id = 2000            # a hero record (ghero)
//! name = "Hero"
//! [records.attrs]
//! "16" = 999           # max HP
//! ```
//! Records are matched by `id`; unknown ids are appended (needs `kind`); `remove = true` deletes.
use crate::g7f::{self, Record};
use crate::{Error, Result};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Table {
    pub name: String,
    pub records: Vec<Record>,
}

#[derive(Debug, Deserialize, Default)]
pub struct TableOverride {
    #[serde(default)]
    pub records: Vec<RecordPatch>,
}

#[derive(Debug, Deserialize)]
pub struct RecordPatch {
    pub id: i32,
    pub kind: Option<u32>,
    pub name: Option<String>,
    pub desc: Option<String>,
    #[serde(default)]
    pub attrs: BTreeMap<String, i32>,
    #[serde(default)]
    pub remove: bool,
}

impl Table {
    pub fn from_dat(name: &str, bytes: &[u8]) -> Self {
        let (recs, _) = g7f::parse(bytes, 0x200);
        Self { name: name.to_string(), records: recs.iter().map(|r| g7f::friendly(r)).collect() }
    }

    pub fn get(&self, id: i32) -> Option<&Record> {
        self.records.iter().find(|r| r.id == Some(id))
    }

    pub fn parse_override(text: &str, is_toml: bool) -> Result<TableOverride> {
        if is_toml {
            toml::from_str(text).map_err(|e| Error::Config(format!("table override toml: {e}")))
        } else {
            serde_json::from_str(text).map_err(|e| Error::Config(format!("table override json: {e}")))
        }
    }

    pub fn apply(&mut self, ov: &TableOverride) -> Result<()> {
        for p in &ov.records {
            if p.remove {
                self.records.retain(|r| r.id != Some(p.id));
                continue;
            }
            let idx = match self.records.iter().position(|r| r.id == Some(p.id)) {
                Some(i) => i,
                None => {
                    let kind = p.kind.ok_or_else(|| Error::Config(format!("new record {} needs `kind`", p.id)))?;
                    self.records.push(Record { kind, id: Some(p.id), ..Default::default() });
                    self.records.len() - 1
                }
            };
            let r = &mut self.records[idx];
            if let Some(n) = &p.name {
                r.name = Some(n.clone());
            }
            if let Some(d) = &p.desc {
                r.desc = Some(d.clone());
            }
            for (k, v) in &p.attrs {
                let k: i32 = k.parse().map_err(|_| Error::Config(format!("attr key {k:?} is not an integer")))?;
                r.attrs.insert(k, *v);
            }
        }
        Ok(())
    }
}