//! A tiny, generic behavior tree.
//!
//! Nodes tick against a mutable blackboard `B` and return a [`Status`]. This is
//! deliberately minimal: the three classic composites (Selector, Sequence),
//! plus leaf `Condition` and `Action` nodes that call plain functions on the
//! blackboard. It carries no game types itself — `ai.rs` supplies the blackboard
//! and the leaf functions — so the tree structure stays readable and testable.

/// Result of ticking a node.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Success,
    Failure,
    Running,
}

/// A behavior-tree node parameterised over a blackboard type `B`.
pub enum Node<B> {
    /// Try children in order; return the first non-Failure (Success/Running).
    /// Fails only if every child fails. This is the "priority" composite.
    Selector(Vec<Node<B>>),
    /// Run children in order; stop at the first non-Success. Succeeds only if all
    /// children succeed.
    Sequence(Vec<Node<B>>),
    /// Leaf test: Success if the predicate holds, else Failure.
    Condition(fn(&mut B) -> bool),
    /// Leaf action: returns its own Status (Success/Running/Failure). The
    /// name is what `tick_traced` reports, so a tick can be read back.
    Action(&'static str, fn(&mut B) -> Status),
}

impl<B> Node<B> {
    /// Tick this node against the blackboard, returning its status.
    pub fn tick(&self, bb: &mut B) -> Status {
        let mut last = None;
        self.tick_traced(bb, &mut last)
    }

    /// `tick`, also recording in `last_action` the name of the last
    /// `Action` leaf that returned Success or Running - "what the tree
    /// decided this tick" for inspection tools.
    pub fn tick_traced(&self, bb: &mut B, last_action: &mut Option<&'static str>) -> Status {
        match self {
            Node::Selector(children) => {
                for child in children {
                    match child.tick_traced(bb, last_action) {
                        Status::Failure => continue,
                        other => return other, // Success or Running short-circuits
                    }
                }
                Status::Failure
            }
            Node::Sequence(children) => {
                for child in children {
                    match child.tick_traced(bb, last_action) {
                        Status::Success => continue,
                        other => return other, // Failure or Running short-circuits
                    }
                }
                Status::Success
            }
            Node::Condition(pred) => {
                if pred(bb) {
                    Status::Success
                } else {
                    Status::Failure
                }
            }
            Node::Action(name, act) => {
                let status = act(bb);
                if status != Status::Failure {
                    *last_action = Some(name);
                }
                status
            }
        }
    }
}

/// Convenience constructors so tree definitions read cleanly.
pub fn selector<B>(children: Vec<Node<B>>) -> Node<B> {
    Node::Selector(children)
}
pub fn sequence<B>(children: Vec<Node<B>>) -> Node<B> {
    Node::Sequence(children)
}
pub fn condition<B>(pred: fn(&mut B) -> bool) -> Node<B> {
    Node::Condition(pred)
}
pub fn action<B>(name: &'static str, act: fn(&mut B) -> Status) -> Node<B> {
    Node::Action(name, act)
}
