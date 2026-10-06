// SPDX-License-Identifier: MIT

//! The depth-first page-tree traversal shared by PDF inspection and fragment
//! validation.
//!
//! The walk only orders the visits and checks each `/Count`. Callers load
//! and classify every node themselves, because inspection reads objects from
//! the source as it goes while fragment validation uses a prepared index.

use super::types::PdfRef;
use crate::Result;

/// A page-tree node to visit next.
#[derive(Clone, Copy)]
pub(super) struct PageNode {
    pub(super) reference: PdfRef,
    /// The node whose `/Kids` named this one; `None` for the root.
    pub(super) parent: Option<PdfRef>,
    /// Whether an ancestor has a `/MediaBox`.
    pub(super) inherited_media_box: bool,
}

/// One pending step of the walk.
#[derive(Clone, Copy)]
pub(super) enum PageStep {
    Enter(PageNode),
    /// The end of a Pages node's subtree, where its `/Count` is checked.
    Exit {
        reference: PdfRef,
        declared_count: u32,
        first_leaf: usize,
    },
}

/// Visits a page tree depth-first, in document order.
///
/// Each pending step is pushed through the caller's `push`, which bounds the
/// stack its own way.
pub(super) struct PageWalk {
    stack: Vec<PageStep>,
}

impl PageWalk {
    /// Start a walk at `root` with an empty, caller-reserved `stack`.
    pub(super) fn new(
        root: PdfRef,
        mut stack: Vec<PageStep>,
        push: impl FnOnce(&mut Vec<PageStep>, PageStep) -> Result<()>,
    ) -> Result<Self> {
        push(
            &mut stack,
            PageStep::Enter(PageNode {
                reference: root,
                parent: None,
                inherited_media_box: false,
            }),
        )?;
        Ok(Self { stack })
    }

    /// The next node to visit, given the number of leaves found so far.
    ///
    /// A Pages node whose `/Count` differs from the leaves found below it is
    /// returned as `Err(reference)` when its subtree ends.
    pub(super) fn next(&mut self, leaves: usize) -> Option<std::result::Result<PageNode, PdfRef>> {
        loop {
            match self.stack.pop()? {
                PageStep::Enter(node) => return Some(Ok(node)),
                PageStep::Exit {
                    reference,
                    declared_count,
                    first_leaf,
                } => {
                    if leaves.saturating_sub(first_leaf) != declared_count as usize {
                        return Some(Err(reference));
                    }
                }
            }
        }
    }

    /// Schedule the `kids` of Pages node `reference`, in order, followed by
    /// the check of its `/Count`. `media_box` says whether the node or an
    /// ancestor has a `/MediaBox`; `leaves` counts the leaves found so far.
    pub(super) fn push_kids(
        &mut self,
        reference: PdfRef,
        declared_count: u32,
        kids: &[PdfRef],
        media_box: bool,
        leaves: usize,
        mut push: impl FnMut(&mut Vec<PageStep>, PageStep) -> Result<()>,
    ) -> Result<()> {
        push(
            &mut self.stack,
            PageStep::Exit {
                reference,
                declared_count,
                first_leaf: leaves,
            },
        )?;
        for kid in kids.iter().rev() {
            push(
                &mut self.stack,
                PageStep::Enter(PageNode {
                    reference: *kid,
                    parent: Some(reference),
                    inherited_media_box: media_box,
                }),
            )?;
        }
        Ok(())
    }
}
