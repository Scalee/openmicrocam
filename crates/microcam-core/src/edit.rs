//! A commit's operations on the scene: the whole tree replaced, or nodes
//! named by their ids inserted, set, patched or removed. Applied in order to a copy of the scene; the result is checked
//! whole by [`Scene::validate`] before it replaces the scene, so a commit is taken whole or not at all.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::scene::{Kind, Node, Scene};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum Op {
    /// The whole tree, bottom to top.
    Replace { nodes: Vec<Node> },
    /// A new node among the children of group `parent` (the top level without one), at position `at` from the bottom
    /// (on top without one).
    Insert {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<usize>,
        node: Node,
    },
    /// The node with this node's id replaced whole, children included.
    Set { node: Node },
    /// Fields of the node `id` changed (a JSON merge patch one level deep: a field given replaces the node's, `null`
    /// takes an optional one away); `id` itself stays.
    Patch { id: String, fields: Map<String, Value> },
    /// The node `id` and everything under it taken away.
    Remove { id: String },
}

/// `ops` applied in order to a copy of `scene`; the first that cannot be applied refuses the lot, by its index.
pub fn apply(scene: &Scene, ops: &[Op]) -> Result<Scene, String> {
    let mut s = scene.clone();
    for (i, op) in ops.iter().enumerate() {
        one(&mut s.nodes, op).map_err(|e| format!("op {i}: {e}"))?;
    }
    Ok(s)
}

fn one(nodes: &mut Vec<Node>, op: &Op) -> Result<(), String> {
    match op {
        Op::Replace { nodes: new } => *nodes = new.clone(),
        Op::Insert { parent, at, node } => {
            let list = match parent {
                None => nodes,
                Some(p) => match find(nodes, p) {
                    Some((list, k)) => match &mut list[k].kind {
                        Kind::Group(g) => &mut g.children,
                        _ => return Err(format!("parent '{p}' is not a group")),
                    },
                    None => return Err(format!("no node '{p}'")),
                },
            };
            let at = at.unwrap_or(list.len());
            if at > list.len() {
                return Err(format!("position {at} past the {} children", list.len()));
            }
            list.insert(at, node.clone());
        }
        Op::Set { node } => {
            let Some(id) = &node.id else {
                return Err("set: the node has no id".into());
            };
            let (list, k) = find(nodes, id).ok_or_else(|| format!("no node '{id}'"))?;
            list[k] = node.clone();
        }
        Op::Patch { id, fields } => {
            if fields.contains_key("id") {
                return Err("patch: a node's id does not change".into());
            }
            let (list, k) = find(nodes, id).ok_or_else(|| format!("no node '{id}'"))?;
            let Value::Object(mut obj) = serde_json::to_value(&list[k]).map_err(|e| e.to_string())? else {
                unreachable!("a node serialises to an object")
            };
            for (f, v) in fields {
                if v.is_null() {
                    obj.remove(f);
                } else {
                    obj.insert(f.clone(), v.clone());
                }
            }
            list[k] = serde_json::from_value(Value::Object(obj)).map_err(|e| format!("patch '{id}': {e}"))?;
        }
        Op::Remove { id } => {
            let (list, k) = find(nodes, id).ok_or_else(|| format!("no node '{id}'"))?;
            list.remove(k);
        }
    }
    Ok(())
}

/// The list holding the node `id` and its index there, searching groups' children (not masks: a mask changes with its
/// group's `mask` field).
fn find<'a>(nodes: &'a mut Vec<Node>, id: &str) -> Option<(&'a mut Vec<Node>, usize)> {
    if let Some(k) = nodes.iter().position(|n| n.id.as_deref() == Some(id)) {
        return Some((nodes, k));
    }
    for n in nodes.iter_mut() {
        if let Kind::Group(g) = &mut n.kind {
            if let Some(hit) = find(&mut g.children, id) {
                return Some(hit);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(j: &str) -> Scene {
        serde_json::from_str(j).unwrap()
    }

    fn ops(j: &str) -> Vec<Op> {
        serde_json::from_str(j).unwrap()
    }

    const BASE: &str = r##"{"version":2,"nodes":[
        {"id":"panel","kind":"group","x":10,"children":[
            {"id":"bg","kind":"rect","x":0,"y":0,"w":100,"h":50,"fill":"#202020"},
            {"id":"label","kind":"text","x":4,"y":30,"size":20,"text":"Zoom","fill":"#ffffff"}]},
        {"id":"cursor","kind":"group","x":5,"y":5,"window":"own","children":[]}]}"##;

    #[test]
    fn patch_moves_and_hides_and_retexts() {
        let s = apply(&scene(BASE), &ops(r#"[{"op":"patch","id":"cursor","fields":{"x":300,"y":200}},
            {"op":"patch","id":"panel","fields":{"visible":false}},
            {"op":"patch","id":"label","fields":{"text":"Зум"}}]"#))
        .unwrap();
        s.validate().unwrap();
        let Kind::Group(c) = &s.nodes[1].kind else { panic!() };
        assert_eq!((c.x, c.y), (300.0, 200.0));
        assert!(!s.nodes[0].visible);
        let Kind::Group(p) = &s.nodes[0].kind else { panic!() };
        let Kind::Text(t) = &p.children[1].kind else { panic!() };
        assert_eq!(t.text, "Зум");
    }

    #[test]
    fn insert_set_remove() {
        let s = apply(&scene(BASE), &ops(r##"[{"op":"insert","parent":"panel","at":1,"node":{"id":"bar","kind":"rect","x":0,"y":40,"w":100,"h":4,"fill":"#ff8000"}},
            {"op":"insert","node":{"id":"toast","kind":"text","x":0,"y":0,"size":10,"text":"ok","fill":"#ffffff"}},
            {"op":"set","node":{"id":"bg","kind":"ellipse","cx":50,"cy":25,"rx":50,"ry":25,"fill":"#000000"}},
            {"op":"remove","id":"label"}]"##))
        .unwrap();
        s.validate().unwrap();
        let Kind::Group(p) = &s.nodes[0].kind else { panic!() };
        let ids: Vec<_> = p.children.iter().map(|n| n.id.clone().unwrap()).collect();
        assert_eq!(ids, ["bg", "bar"]);
        assert!(matches!(p.children[0].kind, Kind::Ellipse(_)));
        assert_eq!(s.nodes[2].id.as_deref(), Some("toast"));
    }

    #[test]
    fn a_bad_op_refuses_the_lot() {
        let base = scene(BASE);
        let e = apply(&base, &ops(r#"[{"op":"patch","id":"cursor","fields":{"x":1}},{"op":"remove","id":"nope"}]"#)).unwrap_err();
        assert_eq!(e, "op 1: no node 'nope'");
        assert!(apply(&base, &ops(r#"[{"op":"patch","id":"bg","fields":{"id":"x"}}]"#)).is_err());
        assert!(apply(&base, &ops(r#"[{"op":"patch","id":"bg","fields":{"w":"wide"}}]"#)).is_err());
        assert!(apply(&base, &ops(r#"[{"op":"insert","parent":"bg","node":{"kind":"group"}}]"#)).is_err());
        // applied, but a twice-used id is the scene's check
        let s = apply(&base, &ops(r#"[{"op":"insert","node":{"id":"bg","kind":"group"}}]"#)).unwrap();
        assert!(s.validate().is_err());
    }

    #[test]
    fn replace_and_patch_away_an_option() {
        let s = apply(&scene(BASE), &ops(r#"[{"op":"patch","id":"panel","fields":{"clip":{"x":0,"y":0,"w":5,"h":5}}}]"#)).unwrap();
        let Kind::Group(p) = &s.nodes[0].kind else { panic!() };
        assert!(p.clip.is_some());
        let s = apply(&s, &ops(r#"[{"op":"patch","id":"panel","fields":{"clip":null}}]"#)).unwrap();
        let Kind::Group(p) = &s.nodes[0].kind else { panic!() };
        assert!(p.clip.is_none());
        let s = apply(&s, &ops(r#"[{"op":"replace","nodes":[]}]"#)).unwrap();
        assert!(s.nodes.is_empty());
    }
}
