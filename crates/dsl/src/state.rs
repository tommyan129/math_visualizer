//! Resolved state model of one workspace. This is what the browser receives
//! (never raw statements), and what the request log stores as applied results.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ast::{Expr, Ident};

/// Identifier of the stage-1 default scene.
pub const DEFAULT_SCENE: &str = "2D(x, y)";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Function {
    pub key: String,
    pub name: Ident,
    pub inputs: Vec<Ident>,
    pub output: Ident,
    /// Constant keys in signature order (after `;`). Complete: includes every
    /// constant used by functions this one calls (design.md "참조한 함수의 상수").
    pub consts: Vec<String>,
    pub body: Expr,
    /// Body as written, for DSL-notation responses.
    pub body_src: String,
    /// Direct callees (user functions only).
    pub calls: Vec<String>,
    /// Data color index, fixed at creation.
    pub color: u32,
    /// Creation order, for listing.
    #[ts(type = "number")]
    pub order: u64,
    /// Log sequence of the last change; the browser recomputes only changed objects.
    #[ts(type = "number")]
    pub version: u64,
}

impl Function {
    /// `f(x; a, b) -> y`
    pub fn signature(&self) -> String {
        let inputs = self.inputs.iter().map(|i| i.key.as_str()).collect::<Vec<_>>().join(", ");
        let consts = if self.consts.is_empty() { String::new() } else { format!("; {}", self.consts.join(", ")) };
        format!("{}({inputs}{consts}) -> {}", self.key, self.output.key)
    }

    /// Full definition line in DSL notation.
    pub fn source(&self) -> String {
        format!("{}: {}", self.signature(), self.body_src)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Constant {
    pub key: String,
    pub name: Ident,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    /// Integer-only (used in a `sum`/`prod` bound).
    pub integer: bool,
    #[ts(type = "number")]
    pub order: u64,
    #[ts(type = "number")]
    pub version: u64,
}

impl Constant {
    pub fn auto(name: Ident, integer: bool, order: u64) -> Self {
        let (value, min, max, step) = if integer { (5.0, 0.0, 20.0, 1.0) } else { (1.0, -10.0, 10.0, 0.1) };
        Constant { key: name.key.clone(), name, value, min, max, step, integer, order, version: 0 }
    }

    pub fn make_integer(&mut self) {
        self.integer = true;
        self.step = 1.0;
        self.min = self.min.round();
        self.max = self.max.round().max(self.min + 1.0);
        self.value = self.value.round().clamp(self.min, self.max);
    }
}

/// Per-scene user state. Stage 1 has only the default scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SceneState {
    pub key: String,
    /// Functions the user turned off.
    pub hidden: Vec<String>,
    #[ts(type = "number")]
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct State {
    pub functions: BTreeMap<String, Function>,
    pub constants: BTreeMap<String, Constant>,
    pub scenes: BTreeMap<String, SceneState>,
    pub counters: Counters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Counters {
    #[ts(type = "number")]
    pub next_order: u64,
    pub next_color: u32,
}

impl Default for State {
    fn default() -> Self {
        let mut scenes = BTreeMap::new();
        scenes.insert(DEFAULT_SCENE.to_string(), SceneState { key: DEFAULT_SCENE.to_string(), hidden: vec![], version: 0 });
        State { functions: BTreeMap::new(), constants: BTreeMap::new(), scenes, counters: Counters::default() }
    }
}

/// One element of an applied result: the resolved state of a changed object.
/// Replaying changes from a checkpoint reproduces the state without re-parsing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
#[allow(clippy::large_enum_variant)]
pub enum Change {
    PutFunction { function: Function },
    RemoveFunction { key: String },
    PutConstant { constant: Constant },
    RemoveConstant { key: String },
    PutScene { scene: SceneState },
    SetCounters { counters: Counters },
}

impl State {
    pub fn apply_changes(&mut self, changes: &[Change]) {
        for c in changes {
            match c {
                Change::PutFunction { function } => {
                    self.functions.insert(function.key.clone(), function.clone());
                }
                Change::RemoveFunction { key } => {
                    self.functions.remove(key);
                }
                Change::PutConstant { constant } => {
                    self.constants.insert(constant.key.clone(), constant.clone());
                }
                Change::RemoveConstant { key } => {
                    self.constants.remove(key);
                }
                Change::PutScene { scene } => {
                    self.scenes.insert(scene.key.clone(), scene.clone());
                }
                Change::SetCounters { counters } => self.counters = *counters,
            }
        }
    }

    /// Changes turning `old` into `self`; stamps `seq` as the version of every changed object.
    pub fn diff_from(&mut self, old: &State, seq: u64) -> Vec<Change> {
        let mut out = vec![];
        for (k, f) in self.functions.iter_mut() {
            let changed = old.functions.get(k).is_none_or(|o| !same_ignoring_version(o, f, |x| &mut x.version));
            if changed {
                f.version = seq;
                out.push(Change::PutFunction { function: f.clone() });
            } else {
                f.version = old.functions[k].version;
            }
        }
        for k in old.functions.keys().filter(|k| !self.functions.contains_key(*k)) {
            out.push(Change::RemoveFunction { key: k.clone() });
        }
        for (k, c) in self.constants.iter_mut() {
            let changed = old.constants.get(k).is_none_or(|o| !same_ignoring_version(o, c, |x| &mut x.version));
            if changed {
                c.version = seq;
                out.push(Change::PutConstant { constant: c.clone() });
            } else {
                c.version = old.constants[k].version;
            }
        }
        for k in old.constants.keys().filter(|k| !self.constants.contains_key(*k)) {
            out.push(Change::RemoveConstant { key: k.clone() });
        }
        for (k, s) in self.scenes.iter_mut() {
            let changed = old.scenes.get(k).is_none_or(|o| !same_ignoring_version(o, s, |x| &mut x.version));
            if changed {
                s.version = seq;
                out.push(Change::PutScene { scene: s.clone() });
            } else {
                s.version = old.scenes[k].version;
            }
        }
        if self.counters != old.counters {
            out.push(Change::SetCounters { counters: self.counters });
        }
        out
    }
}

fn same_ignoring_version<T: Clone + PartialEq>(old: &T, new: &T, version: impl Fn(&mut T) -> &mut u64) -> bool {
    let mut a = old.clone();
    *version(&mut a) = 0;
    let mut b = new.clone();
    *version(&mut b) = 0;
    a == b
}
