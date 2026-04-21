use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct EdgeCondition {
    pub op: String,
    pub input: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone)]
pub struct EdgeVideos {
    #[serde(default)]
    pub webp: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(rename = "isLoop")]
    pub is_loop: bool,
    pub videos: EdgeVideos,
    pub duration: f32,
    #[serde(default)]
    pub conditions: Vec<EdgeCondition>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Pack {
    #[serde(rename = "initialNode")]
    pub initial_node: String,
    pub edges: Vec<Edge>,
}

impl Pack {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let text = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn edge(&self, id: &str) -> Option<&Edge> {
        self.edges.iter().find(|e| e.id == id)
    }
}

/// Resolve an edge's webp path on disk. The pack JSON stores URLs like
/// `/packs/clippy-masko/<id>.webp` relative to the project's `public/` dir.
pub fn webp_path_for(project_root: &Path, edge: &Edge) -> Option<PathBuf> {
    let url = edge.videos.webp.as_deref()?;
    let rel = url.trim_start_matches('/');
    Some(project_root.join("public").join(rel))
}
