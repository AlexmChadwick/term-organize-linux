//! What the course page draws for a prerequisite or corequisite.
//! Port of `RequisiteOutline.swift`.

use crate::model::{CourseKey, Requisite};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    Course(CourseKey),
    Test { name: String, score: Option<f64> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct RequisiteItem {
    pub kind: ItemKind,
    pub is_complete: bool,
    pub via: Option<CourseKey>,
    pub is_offered: bool,
}

impl RequisiteItem {
    pub fn course_key(&self) -> Option<&CourseKey> {
        match &self.kind {
            ItemKind::Course(key) => Some(key),
            ItemKind::Test { .. } => None,
        }
    }
    pub fn is_greyed_out(&self) -> bool {
        self.course_key().is_some() && !self.is_offered && !self.is_complete
    }
    pub fn score_text(&self) -> Option<String> {
        let ItemKind::Test {
            name,
            score: Some(score),
        } = &self.kind
        else {
            return None;
        };
        let text = if (score - score.round()).abs() < 1e-9 {
            format!("{}", *score as i64)
        } else {
            let tenths = (score * 100.0).round() / 100.0;
            if (tenths - tenths.round()).abs() < 1e-9 {
                format!("{}", tenths as i64)
            } else {
                format!("{tenths}")
            }
        };
        if name.contains(&text) {
            None
        } else {
            Some(text)
        }
    }
    pub fn spoken_text(&self) -> String {
        match &self.kind {
            ItemKind::Course(key) => {
                if !self.is_complete {
                    key.to_string()
                } else if let Some(via) = &self.via {
                    format!("{key} (completed via {via})")
                } else {
                    format!("{key} (completed)")
                }
            }
            ItemKind::Test { name, .. } => match self.score_text() {
                Some(score) => format!("{name} (minimum score {score})"),
                None => name.clone(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RequisiteNode {
    Item(RequisiteItem),
    All(Vec<RequisiteNode>),
    OneOf(Vec<RequisiteNode>),
}

impl RequisiteNode {
    pub fn all_items(&self) -> Vec<RequisiteItem> {
        match self {
            Self::Item(item) => vec![item.clone()],
            Self::All(members) | Self::OneOf(members) => {
                members.iter().flat_map(|m| m.all_items()).collect()
            }
        }
    }
    pub fn is_done(&self) -> bool {
        match self {
            Self::Item(item) => item.is_complete,
            Self::All(members) => members.iter().all(|m| m.is_done()),
            Self::OneOf(members) => members.iter().any(|m| m.is_done()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupKind {
    All,
    OneOf,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RequisiteGroup {
    pub kind: GroupKind,
    pub items: Vec<RequisiteItem>,
}

impl RequisiteGroup {
    pub fn is_done(&self) -> bool {
        match self.kind {
            GroupKind::All => self.items.iter().all(|i| i.is_complete),
            GroupKind::OneOf => self.items.iter().any(|i| i.is_complete),
        }
    }
    pub fn done_with(&self) -> Option<CourseKey> {
        if self.kind != GroupKind::OneOf {
            return None;
        }
        let done = self.items.iter().find(|i| i.is_complete)?;
        done.via.clone().or_else(|| done.course_key().cloned())
    }
    pub fn spoken_text(&self) -> String {
        let names: Vec<String> = self.items.iter().map(|i| i.spoken_text()).collect();
        match self.kind {
            GroupKind::OneOf => format!("one of {}", spoken_list(&names, "or")),
            GroupKind::All => {
                if names.len() == 2 {
                    format!("both {} and {}", names[0], names[1])
                } else {
                    format!("all of {}", spoken_list(&names, "and"))
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RequisitePart {
    Item(RequisiteItem),
    Group(RequisiteGroup),
    Expression(RequisiteNode),
}

impl RequisitePart {
    pub fn is_done(&self) -> bool {
        match self {
            Self::Item(item) => item.is_complete,
            Self::Group(group) => group.is_done(),
            Self::Expression(node) => node.is_done(),
        }
    }
    pub fn spoken_text(&self) -> String {
        match self {
            Self::Item(item) => item.spoken_text(),
            Self::Group(group) => group.spoken_text(),
            Self::Expression(node) => spoken_expression(node),
        }
    }
    fn is_expression(&self) -> bool {
        matches!(self, Self::Expression(_))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RequisiteChecklist {
    pub requirement_count: usize,
    pub parts: Vec<RequisitePart>,
}

impl RequisiteChecklist {
    pub fn from_members(members: &[RequisiteNode]) -> Self {
        let members = flatten(members, false);
        let mut parts = Vec::new();
        let mut loose_index = None;
        let mut loose = Vec::new();
        for member in &members {
            match member {
                RequisiteNode::Item(item) => {
                    if loose_index.is_none() {
                        loose_index = Some(parts.len());
                        parts.push(RequisitePart::Item(item.clone()));
                    }
                    loose.push(item.clone());
                }
                RequisiteNode::OneOf(alternatives) => {
                    if let Some(items) = item_members(alternatives) {
                        parts.push(RequisitePart::Group(RequisiteGroup {
                            kind: GroupKind::OneOf,
                            items: choice_order(items),
                        }));
                    } else {
                        parts.push(RequisitePart::Expression(member.clone()));
                    }
                }
                RequisiteNode::All(_) => parts.push(RequisitePart::Expression(member.clone())),
            }
        }
        if let Some(index) = loose_index {
            parts[index] = if loose.len() == 1 {
                RequisitePart::Item(loose[0].clone())
            } else {
                RequisitePart::Group(RequisiteGroup {
                    kind: GroupKind::All,
                    items: loose,
                })
            };
        }
        Self {
            requirement_count: members.len(),
            parts,
        }
    }
    pub fn is_done(&self) -> bool {
        self.parts.iter().all(|p| p.is_done())
    }
    pub fn spoken_text(&self) -> String {
        let separator = if self.parts.iter().any(|p| p.is_expression()) {
            "; and "
        } else {
            ", and "
        };
        self.parts
            .iter()
            .map(|p| p.spoken_text())
            .collect::<Vec<_>>()
            .join(separator)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PathContent {
    Item(RequisiteItem),
    Group(RequisiteGroup),
    Checklist(RequisiteChecklist),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RequisitePath {
    pub content: PathContent,
}

impl RequisitePath {
    pub fn paths(members: &[RequisiteNode]) -> Vec<Self> {
        let mut paths = Vec::new();
        let mut loose_index = None;
        let mut loose = Vec::new();
        for member in flatten(members, true) {
            match &member {
                RequisiteNode::Item(item) => {
                    if loose_index.is_none() {
                        loose_index = Some(paths.len());
                        paths.push(RequisitePath {
                            content: PathContent::Item(item.clone()),
                        });
                    }
                    loose.push(item.clone());
                }
                RequisiteNode::All(inner) => {
                    if let Some(items) = item_members(inner) {
                        paths.push(RequisitePath {
                            content: PathContent::Group(RequisiteGroup {
                                kind: GroupKind::All,
                                items,
                            }),
                        });
                    } else {
                        paths.push(RequisitePath {
                            content: PathContent::Checklist(RequisiteChecklist::from_members(
                                inner,
                            )),
                        });
                    }
                }
                RequisiteNode::OneOf(_) => {}
            }
        }
        if let Some(index) = loose_index {
            paths[index].content = if loose.len() == 1 {
                PathContent::Item(loose[0].clone())
            } else {
                PathContent::Group(RequisiteGroup {
                    kind: GroupKind::OneOf,
                    items: choice_order(loose),
                })
            };
        }
        paths
    }
    pub fn is_done(&self) -> bool {
        match &self.content {
            PathContent::Item(item) => item.is_complete,
            PathContent::Group(group) => group.is_done(),
            PathContent::Checklist(list) => list.is_done(),
        }
    }
    pub fn spoken_text(&self) -> String {
        match &self.content {
            PathContent::Item(item) => item.spoken_text(),
            PathContent::Group(group) => group.spoken_text(),
            PathContent::Checklist(list) => list.spoken_text(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RequisiteLayout {
    Item(RequisiteItem),
    Group(RequisiteGroup),
    Checklist(RequisiteChecklist),
    Paths(Vec<RequisitePath>),
}

impl RequisiteLayout {
    pub fn build(
        requisite: &Requisite,
        is_offered: impl Fn(&CourseKey) -> bool,
        completion: impl Fn(&CourseKey) -> (bool, Option<CourseKey>),
    ) -> Option<Self> {
        let tree = normalized(requisite)?;
        Some(Self::from_node(&make_node(&tree, &is_offered, &completion)))
    }

    pub fn from_node(node: &RequisiteNode) -> Self {
        match node {
            RequisiteNode::Item(item) => Self::Item(item.clone()),
            RequisiteNode::All(members) => {
                let members = flatten(members, false);
                if let Some(items) = item_members(&members) {
                    Self::Group(RequisiteGroup {
                        kind: GroupKind::All,
                        items,
                    })
                } else {
                    Self::Checklist(RequisiteChecklist::from_members(&members))
                }
            }
            RequisiteNode::OneOf(members) => {
                let members = flatten(members, true);
                if let Some(items) = item_members(&members) {
                    Self::Group(RequisiteGroup {
                        kind: GroupKind::OneOf,
                        items: choice_order(items),
                    })
                } else {
                    Self::Paths(RequisitePath::paths(&members))
                }
            }
        }
    }

    pub fn is_done(&self) -> bool {
        match self {
            Self::Item(item) => item.is_complete,
            Self::Group(group) => group.is_done(),
            Self::Checklist(list) => list.is_done(),
            Self::Paths(paths) => paths.iter().any(|p| p.is_done()),
        }
    }

    pub fn spoken_description(&self, title: &str) -> String {
        let body = match self {
            Self::Item(item) => item.spoken_text(),
            Self::Group(group) => group.spoken_text(),
            Self::Checklist(list) => list.spoken_text(),
            Self::Paths(paths) => format!(
                "either {}",
                paths
                    .iter()
                    .map(|p| p.spoken_text())
                    .collect::<Vec<_>>()
                    .join("; or ")
            ),
        };
        format!("{title}: {body}.")
    }
}

pub fn normalized(requisite: &Requisite) -> Option<Requisite> {
    match requisite {
        Requisite::Course(_) | Requisite::Test { .. } => Some(requisite.clone()),
        Requisite::And(items) => merging(items, true),
        Requisite::Or(items) => merging(items, false),
    }
}

fn merging(items: &[Requisite], as_and: bool) -> Option<Requisite> {
    let mut members = Vec::new();
    let mut seen = HashSet::new();
    for item in items {
        let Some(member) = normalized(item) else {
            continue;
        };
        match (&member, as_and) {
            (Requisite::And(inner), true) | (Requisite::Or(inner), false) => {
                for child in inner {
                    push_unique(&mut members, &mut seen, child.clone());
                }
            }
            _ => push_unique(&mut members, &mut seen, member),
        }
    }
    match members.len() {
        0 => None,
        1 => members.pop(),
        _ => Some(if as_and {
            Requisite::And(members)
        } else {
            Requisite::Or(members)
        }),
    }
}

fn push_unique(members: &mut Vec<Requisite>, seen: &mut HashSet<String>, member: Requisite) {
    if seen.insert(display_key(&member)) {
        members.push(member);
    }
}

fn display_key(requisite: &Requisite) -> String {
    match requisite {
        Requisite::Course(key) => format!("c:{key}"),
        Requisite::Test { name, score } => format!(
            "t:{name}:{}",
            score.map(|s| s.to_string()).unwrap_or_default()
        ),
        Requisite::And(items) => {
            let mut keys: Vec<String> = items.iter().map(display_key).collect();
            keys.sort();
            format!("and({})", keys.join(","))
        }
        Requisite::Or(items) => {
            let mut keys: Vec<String> = items.iter().map(display_key).collect();
            keys.sort();
            format!("or({})", keys.join(","))
        }
    }
}

fn make_node(
    requisite: &Requisite,
    is_offered: &impl Fn(&CourseKey) -> bool,
    completion: &impl Fn(&CourseKey) -> (bool, Option<CourseKey>),
) -> RequisiteNode {
    match requisite {
        Requisite::Course(key) => {
            let (is_complete, via) = completion(key);
            RequisiteNode::Item(RequisiteItem {
                kind: ItemKind::Course(key.clone()),
                is_complete,
                via: if is_complete { via } else { None },
                is_offered: is_offered(key),
            })
        }
        Requisite::Test { name, score } => RequisiteNode::Item(RequisiteItem {
            kind: ItemKind::Test {
                name: name.clone(),
                score: *score,
            },
            is_complete: false,
            via: None,
            is_offered: true,
        }),
        Requisite::And(items) => RequisiteNode::All(
            items
                .iter()
                .map(|i| make_node(i, is_offered, completion))
                .collect(),
        ),
        Requisite::Or(items) => RequisiteNode::OneOf(
            items
                .iter()
                .map(|i| make_node(i, is_offered, completion))
                .collect(),
        ),
    }
}

fn flatten(members: &[RequisiteNode], one_of: bool) -> Vec<RequisiteNode> {
    members
        .iter()
        .flat_map(|member| match member {
            RequisiteNode::OneOf(inner) if one_of => flatten(inner, true),
            RequisiteNode::All(inner) if !one_of => flatten(inner, false),
            _ => vec![member.clone()],
        })
        .collect()
}

fn item_members(members: &[RequisiteNode]) -> Option<Vec<RequisiteItem>> {
    let mut items = Vec::new();
    for member in members {
        match member {
            RequisiteNode::Item(item) => items.push(item.clone()),
            _ => return None,
        }
    }
    Some(items)
}

fn choice_order(items: Vec<RequisiteItem>) -> Vec<RequisiteItem> {
    let mut ranked: Vec<(usize, usize, RequisiteItem)> = items
        .into_iter()
        .enumerate()
        .map(|(offset, item)| {
            let rank = if item.is_complete {
                0
            } else if item.course_key().is_some() && item.is_offered {
                1
            } else {
                2
            };
            (rank, offset, item)
        })
        .collect();
    ranked.sort_by_key(|(rank, offset, _)| (*rank, *offset));
    ranked.into_iter().map(|(_, _, item)| item).collect()
}

fn spoken_list(names: &[String], word: &str) -> String {
    let Some(last) = names.last() else {
        return String::new();
    };
    if names.len() == 1 {
        return last.clone();
    }
    format!("{} {word} {last}", names[..names.len() - 1].join(", "))
}

fn spoken_expression(node: &RequisiteNode) -> String {
    match node {
        RequisiteNode::Item(item) => item.spoken_text(),
        RequisiteNode::All(members) => {
            let spoken: Vec<String> = members.iter().map(spoken_expression).collect();
            if spoken.len() == 2 {
                format!("both {} and {}", spoken[0], spoken[1])
            } else {
                format!("all of {}", spoken_list(&spoken, "and"))
            }
        }
        RequisiteNode::OneOf(members) => format!(
            "either {}",
            members
                .iter()
                .map(spoken_expression)
                .collect::<Vec<_>>()
                .join(", or ")
        ),
    }
}
