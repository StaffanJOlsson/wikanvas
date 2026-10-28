//! Tree move validation rules, and the reference model of the tree's soft states (owner: W1).
//!
//! The store applies these rules in SQL inside the writer transaction (§5 "Tree queries", "Trash", "Collection
//! delete"); the triggers of `0001-init` repeat the parent rules as a guard. The rules:
//! - **Parent.** A new parent (create, move) must exist, be live (not trashed) and unarchived, and be in the item's
//!   (new) collection; the collection must be live. Otherwise 409 `parent_invalid`.
//! - **Cycle.** A move under the item itself or under one of its descendants is 409 `cycle`: the target parent's
//!   ancestor chain must not contain the moving item ([`check_move`]).
//! - **Read-only.** Only live, unarchived items are moved, renamed, published or archived (409 `read_only`).
//! - **Trash.** Trashing marks the item and every descendant that is not already trashed with the item as its trash
//!   root; restore clears exactly the rows of that root. Deleting a collection marks its live items with the
//!   collection as their root.
//! - **Archive.** Archiving marks the item and every descendant that is neither archived nor trashed; unarchive
//!   clears exactly the rows of that root.
//! - **Placement on restore and unarchive.** The root keeps its parent unless the parent is gone: purged, trashed, or
//!   archived while the root comes back unarchived ([`keeps_parent`]). Then the root goes to the end of the top level
//!   (`moved_to_top`).
//!
//! These rules keep two invariants that the tree endpoint relies on: the parent of a live item is live, and the parent
//! of a live, unarchived item is unarchived. [`Model`] is an in-memory implementation of the same rules: the proptest
//! below checks that random operations never make a cycle, keep both invariants, and match a brute-force model.

use std::collections::BTreeSet;

/// Why a tree operation is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TreeError {
    /// 409 `cycle`.
    #[error("an item cannot move inside itself")]
    Cycle,
    /// 409 `parent_invalid`.
    #[error("the parent or collection is missing, in the trash, archived or in another collection")]
    ParentInvalid,
    /// 409 `read_only`.
    #[error("the item is in the trash or archived")]
    ReadOnly,
    /// 409 `conflict`: the operation does not apply to the item's state (restoring a live item, purging one that is
    /// not in the trash).
    #[error("the item is not in a state this operation applies to")]
    Conflict,
    /// 404 `not_found`.
    #[error("no such item or collection")]
    NotFound,
}

/// The state of a would-be parent, as the rules see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParentState {
    pub trashed: bool,
    pub archived: bool,
}

/// The cycle rule: `moving` may go under the parent whose ancestor chain is `target_chain` (the target parent first,
/// then its parent, up to the top). `None` (the top level) never cycles.
pub fn check_move<T: PartialEq>(moving: &T, target_chain: Option<&[T]>) -> Result<(), TreeError> {
    match target_chain {
        Some(chain) if chain.contains(moving) => Err(TreeError::Cycle),
        _ => Ok(()),
    }
}

/// The parent rule for create and move: the parent exists (`Some`), is live and unarchived.
pub fn check_new_parent(parent: Option<ParentState>) -> Result<(), TreeError> {
    match parent {
        Some(ParentState {
            trashed: false,
            archived: false,
        }) => Ok(()),
        _ => Err(TreeError::ParentInvalid),
    }
}

/// The placement rule of restore and unarchive: whether the root keeps `parent` (`None`: the parent row is gone),
/// given whether the root itself stays archived afterwards.
pub fn keeps_parent(parent: Option<ParentState>, root_archived: bool) -> bool {
    match parent {
        None => false,
        Some(p) => !p.trashed && (!p.archived || root_archived),
    }
}

/// What trashed an item: itself or an ancestor (an item root), or its collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TrashRoot {
    Item(usize),
    Collection(usize),
}

/// One item of a [`Model`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub collection: usize,
    pub parent: Option<usize>,
    pub trash_root: Option<TrashRoot>,
    pub archive_root: Option<usize>,
    pub purged: bool,
}

/// An in-memory tree with the store's rules: items are numbered in creation order, collections are `0..n`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Model {
    pub nodes: Vec<Node>,
    /// Whether each collection is deleted.
    pub collections_deleted: Vec<bool>,
}

impl Model {
    /// A model with `collections` live collections and no items.
    pub fn new(collections: usize) -> Model {
        Model {
            nodes: Vec::new(),
            collections_deleted: vec![false; collections],
        }
    }

    fn node(&self, id: usize) -> Result<&Node, TreeError> {
        self.nodes
            .get(id)
            .filter(|n| !n.purged)
            .ok_or(TreeError::NotFound)
    }

    fn collection_live(&self, c: usize) -> Result<(), TreeError> {
        match self.collections_deleted.get(c) {
            Some(false) => Ok(()),
            _ => Err(TreeError::ParentInvalid),
        }
    }

    fn parent_state(&self, id: usize) -> Option<ParentState> {
        self.node(id).ok().map(|n| ParentState {
            trashed: n.trash_root.is_some(),
            archived: n.archive_root.is_some(),
        })
    }

    /// The ancestor chain of `id`: its parent first, up to the top.
    pub fn ancestors(&self, id: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut cur = self.nodes.get(id).and_then(|n| n.parent);
        while let Some(p) = cur {
            if out.contains(&p) {
                break; // a corrupt cycle; never produced by the rules
            }
            out.push(p);
            cur = self.nodes.get(p).and_then(|n| n.parent);
        }
        out
    }

    /// `id` and every descendant, parents before children.
    pub fn subtree(&self, id: usize) -> Vec<usize> {
        let mut out = vec![id];
        let mut i = 0;
        while i < out.len() {
            let parent = out[i];
            out.extend(
                (0..self.nodes.len())
                    .filter(|&c| !self.nodes[c].purged && self.nodes[c].parent == Some(parent)),
            );
            i += 1;
        }
        out
    }

    fn writable(&self, id: usize) -> Result<&Node, TreeError> {
        let n = self.node(id)?;
        if n.trash_root.is_some() || n.archive_root.is_some() {
            return Err(TreeError::ReadOnly);
        }
        Ok(n)
    }

    fn check_parent(&self, collection: usize, parent: Option<usize>) -> Result<(), TreeError> {
        self.collection_live(collection)?;
        if let Some(p) = parent {
            check_new_parent(self.parent_state(p))?;
            if self.nodes[p].collection != collection {
                return Err(TreeError::ParentInvalid);
            }
        }
        Ok(())
    }

    /// Creates an item; returns its id.
    pub fn create(&mut self, collection: usize, parent: Option<usize>) -> Result<usize, TreeError> {
        self.check_parent(collection, parent)?;
        self.nodes.push(Node {
            collection,
            parent,
            trash_root: None,
            archive_root: None,
            purged: false,
        });
        Ok(self.nodes.len() - 1)
    }

    /// Moves `id` (with its subtree) under `parent` in `collection`.
    pub fn move_to(
        &mut self,
        id: usize,
        collection: usize,
        parent: Option<usize>,
    ) -> Result<(), TreeError> {
        self.writable(id)?;
        self.check_parent(collection, parent)?;
        if let Some(p) = parent {
            let mut chain = vec![p];
            chain.extend(self.ancestors(p));
            check_move(&id, Some(chain.as_slice()))?;
        }
        for d in self.subtree(id) {
            self.nodes[d].collection = collection;
        }
        self.nodes[id].parent = parent;
        Ok(())
    }

    /// Trashes `id` and its descendants that are not trashed yet; returns how many rows it marked.
    pub fn trash(&mut self, id: usize) -> Result<u64, TreeError> {
        if self.node(id)?.trash_root.is_some() {
            return Err(TreeError::ReadOnly);
        }
        let mut n = 0;
        for d in self.subtree(id) {
            if self.nodes[d].trash_root.is_none() {
                self.nodes[d].trash_root = Some(TrashRoot::Item(id));
                n += 1;
            }
        }
        Ok(n)
    }

    /// Restores the trash unit of `id` (its root); returns the root and whether it went to the top level.
    pub fn restore(&mut self, id: usize) -> Result<(usize, bool), TreeError> {
        let root = match self.node(id)?.trash_root {
            None => return Err(TreeError::Conflict),
            Some(TrashRoot::Collection(_)) => return Err(TreeError::ParentInvalid),
            Some(TrashRoot::Item(r)) => r,
        };
        self.collection_live(self.nodes[root].collection)?;
        for n in self.nodes.iter_mut().filter(|n| !n.purged) {
            if n.trash_root == Some(TrashRoot::Item(root)) {
                n.trash_root = None;
            }
        }
        Ok((root, self.place(root)))
    }

    /// Archives `id` and its descendants that are neither archived nor trashed; returns how many rows it marked.
    pub fn archive(&mut self, id: usize) -> Result<u64, TreeError> {
        self.writable(id)?;
        let mut n = 0;
        for d in self.subtree(id) {
            let node = &mut self.nodes[d];
            if node.archive_root.is_none() && node.trash_root.is_none() {
                node.archive_root = Some(id);
                n += 1;
            }
        }
        Ok(n)
    }

    /// Unarchives the archive unit of `id` (its root); returns the root and whether it went to the top level.
    pub fn unarchive(&mut self, id: usize) -> Result<(usize, bool), TreeError> {
        let node = self.node(id)?;
        if node.trash_root.is_some() {
            return Err(TreeError::ReadOnly);
        }
        let root = node.archive_root.ok_or(TreeError::Conflict)?;
        if self.node(root)?.trash_root.is_some() {
            return Err(TreeError::ReadOnly);
        }
        for n in self.nodes.iter_mut().filter(|n| !n.purged) {
            if n.archive_root == Some(root) {
                n.archive_root = None;
            }
        }
        Ok((root, self.place(root)))
    }

    /// The placement rule after a restore or unarchive of `root`.
    fn place(&mut self, root: usize) -> bool {
        let Some(parent) = self.nodes[root].parent else {
            return false;
        };
        let archived = self.nodes[root].archive_root.is_some();
        if keeps_parent(self.parent_state(parent), archived) {
            return false;
        }
        self.nodes[root].parent = None;
        true
    }

    /// Soft-deletes a collection; returns how many items it trashed.
    pub fn delete_collection(&mut self, c: usize) -> Result<u64, TreeError> {
        if self.collections_deleted.get(c) != Some(&false) {
            return Err(TreeError::NotFound);
        }
        self.collections_deleted[c] = true;
        let mut n = 0;
        for node in self
            .nodes
            .iter_mut()
            .filter(|n| !n.purged && n.collection == c)
        {
            if node.trash_root.is_none() {
                node.trash_root = Some(TrashRoot::Collection(c));
                n += 1;
            }
        }
        Ok(n)
    }

    /// Restores a deleted collection and exactly the items its deletion trashed.
    pub fn restore_collection(&mut self, c: usize) -> Result<(), TreeError> {
        if self.collections_deleted.get(c) != Some(&true) {
            return Err(TreeError::Conflict);
        }
        self.collections_deleted[c] = false;
        for node in self.nodes.iter_mut().filter(|n| !n.purged) {
            if node.trash_root == Some(TrashRoot::Collection(c)) {
                node.trash_root = None;
            }
        }
        Ok(())
    }

    /// Permanently deletes a trashed item and its subtree; returns how many rows went.
    pub fn purge(&mut self, id: usize) -> Result<u64, TreeError> {
        if self.node(id)?.trash_root.is_none() {
            return Err(TreeError::Conflict);
        }
        let gone = self.subtree(id);
        for &d in &gone {
            self.nodes[d].purged = true;
        }
        Ok(gone.len() as u64)
    }

    /// The live, unarchived items (what the tree endpoint lists), with their parents.
    pub fn visible(&self) -> BTreeSet<(usize, usize, Option<usize>)> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                !n.purged
                    && n.trash_root.is_none()
                    && n.archive_root.is_none()
                    && !self.collections_deleted[n.collection]
            })
            .map(|(i, n)| (i, n.collection, n.parent))
            .collect()
    }

    /// Checks the invariants of the module docs; returns the first violation.
    pub fn check_invariants(&self) -> Result<(), String> {
        for (i, n) in self.nodes.iter().enumerate().filter(|(_, n)| !n.purged) {
            let mut seen = BTreeSet::from([i]);
            let mut cur = n.parent;
            while let Some(p) = cur {
                if !seen.insert(p) {
                    return Err(format!("{i}: cycle through {p}"));
                }
                cur = self.nodes[p].parent;
            }
            let Some(p) = n.parent else { continue };
            let parent = &self.nodes[p];
            if parent.purged {
                return Err(format!("{i}: parent {p} is purged"));
            }
            if parent.collection != n.collection {
                return Err(format!("{i}: parent {p} is in another collection"));
            }
            if n.trash_root.is_none() && parent.trash_root.is_some() {
                return Err(format!("{i}: live under trashed {p}"));
            }
            if n.trash_root.is_none() && n.archive_root.is_none() && parent.archive_root.is_some() {
                return Err(format!("{i}: unarchived under archived {p}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::btree_map::Entry;

    use proptest::prelude::*;

    use super::*;

    #[test]
    fn rules() {
        assert_eq!(check_move(&1, Some(&[2, 3, 1][..])), Err(TreeError::Cycle));
        assert_eq!(check_move(&1, Some(&[1][..])), Err(TreeError::Cycle));
        assert_eq!(check_move(&1, Some(&[2, 3][..])), Ok(()));
        assert_eq!(check_move::<u8>(&1, None), Ok(()));
        let live = ParentState {
            trashed: false,
            archived: false,
        };
        let archived = ParentState {
            trashed: false,
            archived: true,
        };
        let trashed = ParentState {
            trashed: true,
            archived: false,
        };
        assert_eq!(check_new_parent(Some(live)), Ok(()));
        for bad in [None, Some(archived), Some(trashed)] {
            assert_eq!(check_new_parent(bad), Err(TreeError::ParentInvalid));
        }
        assert!(keeps_parent(Some(live), false));
        assert!(!keeps_parent(Some(archived), false));
        assert!(keeps_parent(Some(archived), true));
        assert!(!keeps_parent(Some(trashed), true));
        assert!(!keeps_parent(None, false));
    }

    #[test]
    fn restore_under_a_trashed_parent_goes_to_the_top() {
        let mut m = Model::new(1);
        let a = m.create(0, None).unwrap();
        let b = m.create(0, Some(a)).unwrap();
        assert_eq!(m.trash(b), Ok(1));
        assert_eq!(m.trash(a), Ok(1));
        assert_eq!(m.restore(b), Ok((b, true)));
        assert_eq!(m.nodes[b].parent, None);
        assert_eq!(m.move_to(a, 0, None), Err(TreeError::ReadOnly));
        assert_eq!(m.restore(a), Ok((a, false)));
        assert_eq!(m.move_to(a, 0, Some(b)), Ok(()));
        assert_eq!(m.move_to(b, 0, Some(a)), Err(TreeError::Cycle));
        m.check_invariants().unwrap();
    }

    /// A brute-force restatement of the rules, written independently of [`Model`]: flat maps, every query a scan.
    #[derive(Clone, Debug, Default)]
    struct Naive {
        parent: BTreeMap<usize, Option<usize>>,
        collection: BTreeMap<usize, usize>,
        trashed_by: BTreeMap<usize, TrashRoot>,
        archived_by: BTreeMap<usize, usize>,
        deleted_collections: BTreeSet<usize>,
        next: usize,
    }

    impl Naive {
        fn exists(&self, id: usize) -> bool {
            self.parent.contains_key(&id)
        }

        fn is_under(&self, id: usize, ancestor: usize) -> bool {
            let mut cur = Some(id);
            let mut steps = 0;
            while let Some(c) = cur {
                if c == ancestor {
                    return true;
                }
                steps += 1;
                if steps > self.parent.len() {
                    return false;
                }
                cur = self.parent.get(&c).copied().flatten();
            }
            false
        }

        fn below(&self, id: usize) -> Vec<usize> {
            self.parent
                .keys()
                .copied()
                .filter(|&x| self.is_under(x, id))
                .collect()
        }

        fn parent_ok(&self, collection: usize, parent: Option<usize>, ncollections: usize) -> bool {
            collection < ncollections
                && !self.deleted_collections.contains(&collection)
                && parent.is_none_or(|p| {
                    self.exists(p)
                        && self.collection[&p] == collection
                        && !self.trashed_by.contains_key(&p)
                        && !self.archived_by.contains_key(&p)
                })
        }

        fn gone_parent(&self, root: usize) -> bool {
            match self.parent[&root] {
                None => false,
                Some(p) => {
                    !self.exists(p)
                        || self.trashed_by.contains_key(&p)
                        || (self.archived_by.contains_key(&p)
                            && !self.archived_by.contains_key(&root))
                }
            }
        }

        fn apply(&mut self, op: &Op, ncollections: usize) -> Result<u64, TreeError> {
            match *op {
                Op::Create { collection, parent } => {
                    let parent = parent.map(|p| p % self.next.max(1));
                    if !self.parent_ok(collection, parent, ncollections) {
                        return Err(TreeError::ParentInvalid);
                    }
                    let id = self.next;
                    self.next += 1;
                    self.parent.insert(id, parent);
                    self.collection.insert(id, collection);
                    Ok(id as u64)
                }
                Op::Move {
                    item,
                    collection,
                    parent,
                } => {
                    let (item, parent) = (
                        item % self.next.max(1),
                        parent.map(|p| p % self.next.max(1)),
                    );
                    if !self.exists(item) {
                        return Err(TreeError::NotFound);
                    }
                    if self.trashed_by.contains_key(&item) || self.archived_by.contains_key(&item) {
                        return Err(TreeError::ReadOnly);
                    }
                    if !self.parent_ok(collection, parent, ncollections) {
                        return Err(TreeError::ParentInvalid);
                    }
                    if parent.is_some_and(|p| self.is_under(p, item)) {
                        return Err(TreeError::Cycle);
                    }
                    for d in self.below(item) {
                        self.collection.insert(d, collection);
                    }
                    self.parent.insert(item, parent);
                    Ok(0)
                }
                Op::Trash { item } => {
                    let item = item % self.next.max(1);
                    if !self.exists(item) {
                        return Err(TreeError::NotFound);
                    }
                    if self.trashed_by.contains_key(&item) {
                        return Err(TreeError::ReadOnly);
                    }
                    let mut n = 0;
                    for d in self.below(item) {
                        if let Entry::Vacant(e) = self.trashed_by.entry(d) {
                            e.insert(TrashRoot::Item(item));
                            n += 1;
                        }
                    }
                    Ok(n)
                }
                Op::Restore { item } => {
                    let item = item % self.next.max(1);
                    if !self.exists(item) {
                        return Err(TreeError::NotFound);
                    }
                    let root = match self.trashed_by.get(&item) {
                        None => return Err(TreeError::Conflict),
                        Some(TrashRoot::Collection(_)) => return Err(TreeError::ParentInvalid),
                        Some(&TrashRoot::Item(r)) => r,
                    };
                    if self.deleted_collections.contains(&self.collection[&root]) {
                        return Err(TreeError::ParentInvalid);
                    }
                    self.trashed_by.retain(|_, r| *r != TrashRoot::Item(root));
                    let moved = self.gone_parent(root);
                    if moved {
                        self.parent.insert(root, None);
                    }
                    Ok(root as u64 * 2 + u64::from(moved))
                }
                Op::Archive { item } => {
                    let item = item % self.next.max(1);
                    if !self.exists(item) {
                        return Err(TreeError::NotFound);
                    }
                    if self.trashed_by.contains_key(&item) || self.archived_by.contains_key(&item) {
                        return Err(TreeError::ReadOnly);
                    }
                    let mut n = 0;
                    for d in self.below(item) {
                        if !self.archived_by.contains_key(&d) && !self.trashed_by.contains_key(&d) {
                            self.archived_by.insert(d, item);
                            n += 1;
                        }
                    }
                    Ok(n)
                }
                Op::Unarchive { item } => {
                    let item = item % self.next.max(1);
                    if !self.exists(item) {
                        return Err(TreeError::NotFound);
                    }
                    if self.trashed_by.contains_key(&item) {
                        return Err(TreeError::ReadOnly);
                    }
                    let Some(&root) = self.archived_by.get(&item) else {
                        return Err(TreeError::Conflict);
                    };
                    if self.trashed_by.contains_key(&root) {
                        return Err(TreeError::ReadOnly);
                    }
                    self.archived_by.retain(|_, r| *r != root);
                    let moved = self.gone_parent(root);
                    if moved {
                        self.parent.insert(root, None);
                    }
                    Ok(root as u64 * 2 + u64::from(moved))
                }
                Op::DeleteCollection { collection } => {
                    if collection >= ncollections || !self.deleted_collections.insert(collection) {
                        return Err(TreeError::NotFound);
                    }
                    let mut n = 0;
                    for (&id, &c) in &self.collection {
                        if c == collection
                            && let Entry::Vacant(e) = self.trashed_by.entry(id)
                        {
                            e.insert(TrashRoot::Collection(collection));
                            n += 1;
                        }
                    }
                    Ok(n)
                }
                Op::RestoreCollection { collection } => {
                    if !self.deleted_collections.remove(&collection) {
                        return Err(TreeError::Conflict);
                    }
                    self.trashed_by
                        .retain(|_, r| *r != TrashRoot::Collection(collection));
                    Ok(0)
                }
                Op::Purge { item } => {
                    let item = item % self.next.max(1);
                    if !self.exists(item) {
                        return Err(TreeError::NotFound);
                    }
                    if !self.trashed_by.contains_key(&item) {
                        return Err(TreeError::Conflict);
                    }
                    let gone = self.below(item);
                    for d in &gone {
                        self.parent.remove(d);
                        self.collection.remove(d);
                        self.trashed_by.remove(d);
                        self.archived_by.remove(d);
                    }
                    Ok(gone.len() as u64)
                }
            }
        }

        fn visible(&self) -> BTreeSet<(usize, usize, Option<usize>)> {
            self.parent
                .iter()
                .filter(|(id, _)| {
                    !self.trashed_by.contains_key(id)
                        && !self.archived_by.contains_key(id)
                        && !self.deleted_collections.contains(&self.collection[id])
                })
                .map(|(&id, &p)| (id, self.collection[&id], p))
                .collect()
        }
    }

    #[derive(Clone, Debug)]
    enum Op {
        Create {
            collection: usize,
            parent: Option<usize>,
        },
        Move {
            item: usize,
            collection: usize,
            parent: Option<usize>,
        },
        Trash {
            item: usize,
        },
        Restore {
            item: usize,
        },
        Archive {
            item: usize,
        },
        Unarchive {
            item: usize,
        },
        DeleteCollection {
            collection: usize,
        },
        RestoreCollection {
            collection: usize,
        },
        Purge {
            item: usize,
        },
    }

    const COLLECTIONS: usize = 3;

    fn op() -> impl Strategy<Value = Op> {
        let c = 0..COLLECTIONS;
        let i = 0..64usize;
        prop_oneof![
            4 => (c.clone(), prop::option::weighted(0.7, i.clone()))
                .prop_map(|(collection, parent)| Op::Create { collection, parent }),
            4 => (i.clone(), c.clone(), prop::option::weighted(0.7, i.clone()))
                .prop_map(|(item, collection, parent)| Op::Move { item, collection, parent }),
            2 => i.clone().prop_map(|item| Op::Trash { item }),
            2 => i.clone().prop_map(|item| Op::Restore { item }),
            2 => i.clone().prop_map(|item| Op::Archive { item }),
            2 => i.clone().prop_map(|item| Op::Unarchive { item }),
            1 => c.clone().prop_map(|collection| Op::DeleteCollection { collection }),
            1 => c.prop_map(|collection| Op::RestoreCollection { collection }),
            1 => i.prop_map(|item| Op::Purge { item }),
        ]
    }

    /// Applies `op` to the model, with ids reduced the same way the naive model reduces them.
    fn apply(m: &mut Model, op: &Op) -> Result<u64, TreeError> {
        let n = m.nodes.len().max(1);
        match *op {
            Op::Create { collection, parent } => m
                .create(collection, parent.map(|p| p % n))
                .map(|id| id as u64),
            Op::Move {
                item,
                collection,
                parent,
            } => m
                .move_to(item % n, collection, parent.map(|p| p % n))
                .map(|()| 0),
            Op::Trash { item } => m.trash(item % n),
            Op::Restore { item } => m
                .restore(item % n)
                .map(|(r, moved)| r as u64 * 2 + u64::from(moved)),
            Op::Archive { item } => m.archive(item % n),
            Op::Unarchive { item } => m
                .unarchive(item % n)
                .map(|(r, moved)| r as u64 * 2 + u64::from(moved)),
            Op::DeleteCollection { collection } => m.delete_collection(collection),
            Op::RestoreCollection { collection } => m.restore_collection(collection).map(|()| 0),
            Op::Purge { item } => m.purge(item % n),
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(500))]

        #[test]
        fn random_ops_never_cycle_and_match_model(ops in prop::collection::vec(op(), 1..120)) {
            let mut model = Model::new(COLLECTIONS);
            let mut naive = Naive::default();
            for op in &ops {
                let got = apply(&mut model, op);
                let want = naive.apply(op, COLLECTIONS);
                prop_assert_eq!(got, want, "{:?}", op);
                if let Err(e) = model.check_invariants() {
                    return Err(TestCaseError::fail(format!("after {op:?}: {e}")));
                }
                prop_assert_eq!(model.visible(), naive.visible(), "after {:?}", op);
            }
        }
    }
}
