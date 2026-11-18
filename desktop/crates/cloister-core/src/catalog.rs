//! Built-in Blocklists and the related-domain map, loaded from `shared/blocklists.json`.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

const BUILTIN_JSON: &str = include_str!("../../../../shared/blocklists.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlocklistKind {
    Distraction,
    Work,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Blocklist {
    pub id: String,
    pub kind: BlocklistKind,
    pub domains: Vec<String>,
    pub apps: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Catalog {
    pub blocklists: Vec<Blocklist>,
    pub related_domains: BTreeMap<String, Vec<String>>,
}

impl Catalog {
    pub fn builtin() -> &'static Catalog {
        static CATALOG: OnceLock<Catalog> = OnceLock::new();
        CATALOG.get_or_init(|| {
            serde_json::from_str(BUILTIN_JSON).expect("shared/blocklists.json is valid")
        })
    }

    pub fn get(&self, id: &str) -> Option<&Blocklist> {
        self.blocklists.iter().find(|b| b.id == id)
    }

    /// Ids of the Blocklists that make up the Distraction List.
    pub fn distraction_ids(&self) -> impl Iterator<Item = String> + '_ {
        self.blocklists
            .iter()
            .filter(|b| b.kind == BlocklistKind::Distraction)
            .map(|b| b.id.clone())
    }

    /// Ids of the Work List's Blocklists.
    pub fn work_ids(&self) -> impl Iterator<Item = String> + '_ {
        self.blocklists
            .iter()
            .filter(|b| b.kind == BlocklistKind::Work)
            .map(|b| b.id.clone())
    }
}

/// Domains that must be blocked together with `domain` (e.g. `youtube.com` -> `youtu.be`).
pub fn related_domains(domain: &str) -> &'static [String] {
    Catalog::builtin()
        .related_domains
        .get(domain)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
