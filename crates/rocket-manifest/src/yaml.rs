//! A small read-only view of a parsed YAML document, for callers that scan
//! third-party files (`rocket init` reads compose files and Taskfiles) and
//! must not depend on a YAML library themselves.
//!
//! The view sits on the same node tree the manifest decoder uses, so anchors,
//! aliases and `<<` merge keys behave like yaml.v3: aliases are followed
//! transparently and merged keys appear in [`Node::entries`] after the
//! explicit ones (explicit keys win, earlier merges win over later ones).

use crate::node::{Kind, MERGE_TAG, NodeId, Tree, parse_tree};

/// A parsed YAML document (the first one of the stream).
#[derive(Debug)]
pub struct Document {
    tree: Tree,
    root: NodeId,
}

/// What a [`Node`] holds, after following aliases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Scalar,
    Mapping,
    Sequence,
}

/// A node of a [`Document`].
#[derive(Debug, Clone, Copy)]
pub struct Node<'a> {
    tree: &'a Tree,
    id: NodeId,
}

/// Parses the first document of `src`. `Ok(None)` means the stream holds no
/// document at all (an empty file or only comments). The error text follows
/// yaml.v3 (`yaml: line N: ...`).
pub fn parse(src: &str) -> Result<Option<Document>, String> {
    Ok(parse_tree(src)?.map(|(tree, root)| Document { tree, root }))
}

impl Document {
    /// The top-level node.
    pub fn root(&self) -> Node<'_> {
        Node {
            tree: &self.tree,
            id: self.tree.deref(self.root),
        }
    }
}

impl<'a> Node<'a> {
    fn new(tree: &'a Tree, id: NodeId) -> Self {
        Self {
            tree,
            id: tree.deref(id),
        }
    }

    pub fn kind(&self) -> NodeKind {
        match self.tree.get(self.id).kind {
            Kind::Mapping => NodeKind::Mapping,
            Kind::Sequence => NodeKind::Sequence,
            Kind::Scalar | Kind::Alias => NodeKind::Scalar,
        }
    }

    /// The raw text of a scalar (empty for collections).
    pub fn value(&self) -> &'a str {
        &self.tree.get(self.id).value
    }

    /// yaml.v3's short tag: `!!str`, `!!int`, `!!seq`, ...
    pub fn tag(&self) -> &'a str {
        &self.tree.get(self.id).tag
    }

    pub fn line(&self) -> usize {
        self.tree.get(self.id).line
    }

    /// Whether this is an explicit or implicit `null` scalar.
    pub fn is_null(&self) -> bool {
        self.tree.is_null(self.id)
    }

    /// The items of a sequence (empty for anything else).
    pub fn items(&self) -> Vec<Node<'a>> {
        match self.kind() {
            NodeKind::Sequence => self
                .tree
                .get(self.id)
                .content
                .iter()
                .map(|&c| Node::new(self.tree, c))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The key/value pairs of a mapping in document order, with `<<` merges
    /// expanded after the explicit pairs (empty for anything else).
    pub fn entries(&self) -> Vec<(Node<'a>, Node<'a>)> {
        if self.kind() != NodeKind::Mapping {
            return Vec::new();
        }
        let content = &self.tree.get(self.id).content;
        let mut explicit: Vec<(Node<'a>, Node<'a>)> = Vec::new();
        let mut merges: Vec<Node<'a>> = Vec::new();
        for pair in content.as_chunks::<2>().0 {
            let key = Node::new(self.tree, pair[0]);
            let value = Node::new(self.tree, pair[1]);
            if is_merge_key(self.tree, pair[0]) {
                match value.kind() {
                    NodeKind::Mapping => merges.push(value),
                    NodeKind::Sequence => merges.extend(
                        value
                            .items()
                            .into_iter()
                            .filter(|n| n.kind() == NodeKind::Mapping),
                    ),
                    NodeKind::Scalar => {}
                }
            } else {
                explicit.push((key, value));
            }
        }
        for merged in merges {
            for (k, v) in merged.entries() {
                if !explicit.iter().any(|(ek, _)| ek.value() == k.value()) {
                    explicit.push((k, v));
                }
            }
        }
        explicit
    }

    /// The value of the first pair whose key is the scalar `key`.
    pub fn get(&self, key: &str) -> Option<Node<'a>> {
        self.entries()
            .into_iter()
            .find(|(k, _)| k.kind() == NodeKind::Scalar && k.value() == key)
            .map(|(_, v)| v)
    }
}

fn is_merge_key(tree: &Tree, id: NodeId) -> bool {
    let n = tree.get(id);
    n.kind == Kind::Scalar
        && n.value == "<<"
        && (n.tag.is_empty() || n.tag == "!" || n.tag == MERGE_TAG)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_aliases_and_merges() {
        let doc = parse("base: &b {x: 1, y: 2}\nuse:\n  <<: *b\n  y: 3\n  z: 4\n")
            .unwrap()
            .unwrap();
        let used = doc.root().get("use").unwrap();
        let pairs: Vec<(String, String)> = used
            .entries()
            .into_iter()
            .map(|(k, v)| (k.value().to_owned(), v.value().to_owned()))
            .collect();
        assert_eq!(
            pairs,
            [("y", "3"), ("z", "4"), ("x", "1")].map(|(k, v)| (k.to_owned(), v.to_owned()))
        );
    }

    #[test]
    fn empty_stream_has_no_document() {
        assert!(parse("").unwrap().is_none());
        assert!(parse("# only a comment\n").unwrap().is_none());
    }

    #[test]
    fn exposes_kinds_and_tags() {
        let doc = parse("a: [1, two]\nb: ~\n").unwrap().unwrap();
        let root = doc.root();
        assert_eq!(root.kind(), NodeKind::Mapping);
        let a = root.get("a").unwrap();
        assert_eq!(a.kind(), NodeKind::Sequence);
        assert_eq!(a.items()[0].tag(), "!!int");
        assert!(root.get("b").unwrap().is_null());
        assert!(root.get("missing").is_none());
    }
}
