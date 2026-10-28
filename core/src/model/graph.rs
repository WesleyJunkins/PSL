//! The canonical, resolved pipeline graph (docs/04 §Canonical model).

use std::collections::HashMap;
use std::path::PathBuf;

use crate::json::Json;
use crate::model::manifest::{Manifest, PortType};
use crate::model::pipeline::{NodeRuntime, PipelineRuntime};
use crate::syntax::{Kind, Span};

#[derive(Clone, Debug)]
pub struct GInput {
    pub name: String,
    pub index: u16,
    pub ty: PortType,
    /// Stream ids feeding this port, in map order.
    pub sources: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct GOutput {
    pub name: String,
    pub index: u16,
    pub ty: PortType,
    pub stream: String,
}

#[derive(Clone, Debug)]
pub struct GNode {
    pub id: String,
    pub kind: Kind,
    pub package: String,
    pub version: String,
    /// Package directory, absolute.
    pub path: PathBuf,
    /// Package directory relative to the workspace, for display.
    pub display_path: String,
    pub params: Json,
    pub runtime: NodeRuntime,
    pub manifest: Manifest,
    pub inputs: Vec<GInput>,
    pub outputs: Vec<GOutput>,
}

#[derive(Clone, Debug)]
pub struct Edge {
    pub from_node: String,
    pub from_port: String,
    pub to_node: String,
    pub to_port: String,
    pub span: Span,
}

impl Edge {
    pub fn stream(&self) -> String {
        format!("{}.{}", self.from_node, self.from_port)
    }
    pub fn target(&self) -> String {
        format!("{}.{}", self.to_node, self.to_port)
    }
}

#[derive(Clone, Debug)]
pub struct Graph {
    pub name: String,
    pub file: String,
    pub description: String,
    pub runtime: PipelineRuntime,
    pub nodes: Vec<GNode>,
    pub maps: Vec<Edge>,
}

impl Graph {
    pub fn node(&self, id: &str) -> Option<&GNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Order in which to send `start`: outputs, then modules with consumers before
    /// producers, then inputs. Cycles fall back to declaration order (docs/01).
    pub fn start_order(&self) -> Vec<String> {
        let mut order: Vec<String> = self
            .nodes
            .iter()
            .filter(|n| n.kind == Kind::Output)
            .map(|n| n.id.clone())
            .collect();
        // Reverse topological order among modules.
        let modules: Vec<&GNode> = self
            .nodes
            .iter()
            .filter(|n| n.kind == Kind::Module)
            .collect();
        let index: HashMap<&str, usize> = modules
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect();
        let mut consumers_left = vec![0usize; modules.len()]; // number of module consumers not yet started
        let mut producers_of: Vec<Vec<usize>> = vec![Vec::new(); modules.len()];
        for e in &self.maps {
            if let (Some(&a), Some(&b)) = (
                index.get(e.from_node.as_str()),
                index.get(e.to_node.as_str()),
            ) {
                if a != b {
                    consumers_left[a] += 1;
                    producers_of[b].push(a);
                }
            }
        }
        let mut started = vec![false; modules.len()];
        loop {
            let next = (0..modules.len()).find(|&i| !started[i] && consumers_left[i] == 0);
            let i = match next {
                Some(i) => i,
                None => match (0..modules.len()).find(|&i| !started[i]) {
                    Some(i) => i, // cycle: pick the first unstarted
                    None => break,
                },
            };
            started[i] = true;
            order.push(modules[i].id.clone());
            for &p in &producers_of[i] {
                consumers_left[p] = consumers_left[p].saturating_sub(1);
            }
        }
        order.extend(
            self.nodes
                .iter()
                .filter(|n| n.kind == Kind::Input)
                .map(|n| n.id.clone()),
        );
        order
    }

    /// Order in which to send `stop`: the reverse of the start order.
    pub fn stop_order(&self) -> Vec<String> {
        let mut o = self.start_order();
        o.reverse();
        o
    }

    pub fn to_json(&self) -> Json {
        Json::object()
            .with("psl", crate::GRAMMAR_VERSION)
            .with(
                "pipeline",
                Json::object()
                    .with("name", self.name.as_str())
                    .with("file", self.file.as_str())
                    .with("description", self.description.as_str())
                    .with("runtime", self.runtime.to_json()),
            )
            .with(
                "nodes",
                self.nodes
                    .iter()
                    .map(|n| {
                        Json::object()
                            .with("id", n.id.as_str())
                            .with("kind", n.kind.as_str())
                            .with("package", n.package.as_str())
                            .with("version", n.version.as_str())
                            .with("path", n.display_path.as_str())
                            .with("params", n.params.clone())
                            .with("runtime", n.runtime.to_json())
                            .with(
                                "inputs",
                                n.inputs
                                    .iter()
                                    .map(|p| {
                                        Json::object()
                                            .with("name", p.name.as_str())
                                            .with("index", p.index as i64)
                                            .with("type", p.ty.as_str())
                                            .with(
                                                "sources",
                                                p.sources
                                                    .iter()
                                                    .map(|s| Json::Str(s.clone()))
                                                    .collect::<Vec<_>>(),
                                            )
                                    })
                                    .collect::<Vec<_>>(),
                            )
                            .with(
                                "outputs",
                                n.outputs
                                    .iter()
                                    .map(|p| {
                                        Json::object()
                                            .with("name", p.name.as_str())
                                            .with("index", p.index as i64)
                                            .with("type", p.ty.as_str())
                                            .with("stream", p.stream.as_str())
                                    })
                                    .collect::<Vec<_>>(),
                            )
                    })
                    .collect::<Vec<_>>(),
            )
            .with(
                "maps",
                self.maps
                    .iter()
                    .map(|e| {
                        Json::object()
                            .with("from", e.stream())
                            .with("to", e.target())
                    })
                    .collect::<Vec<_>>(),
            )
    }
}
