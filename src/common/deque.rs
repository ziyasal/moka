// License and Copyright Notice:
//
// Some of the code and doc comments in this module were copied from
// `std::collections::LinkedList` in the Rust standard library.
// https://github.com/rust-lang/rust/blob/master/src/liballoc/collections/linked_list.rs
//
// The original code/comments from LinkedList are dual-licensed under
// the Apache License, Version 2.0 <https://github.com/rust-lang/rust/blob/master/LICENSE-APACHE>
// or the MIT license <https://github.com/rust-lang/rust/blob/master/LICENSE-MIT>
//
// Copyrights of the original code/comments are retained by their contributors.
// For full authorship information, see the version control history of
// https://github.com/rust-lang/rust/ or https://thanks.rust-lang.org

use std::{marker::PhantomData, ptr::NonNull};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CacheRegion {
    Window,
    MainProbation,
    MainProtected,
    WriteOrder,
}

#[derive(PartialEq, Eq)]
pub(crate) struct DeqNode<T> {
    pub(crate) region: CacheRegion,
    next: Option<NonNull<DeqNode<T>>>,
    prev: Option<NonNull<DeqNode<T>>>,
    pub(crate) element: T,
}

impl<T> std::fmt::Debug for DeqNode<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeqNode")
            .field("region", &self.region)
            .field("next", &self.next)
            .field("prev", &self.prev)
            .finish()
    }
}

impl<T> DeqNode<T> {
    pub(crate) fn new(region: CacheRegion, element: T) -> Self {
        Self {
            region,
            next: None,
            prev: None,
            element,
        }
    }

    pub(crate) fn next_node(&self) -> Option<&DeqNode<T>> {
        self.next.as_ref().map(|node| unsafe { node.as_ref() })
    }
}

/// Cursor is used to remember the current iterating position.
enum DeqCursor<T> {
    Node(NonNull<DeqNode<T>>),
    Done,
}

pub(crate) struct Deque<T> {
    region: CacheRegion,
    len: usize,
    head: Option<NonNull<DeqNode<T>>>,
    tail: Option<NonNull<DeqNode<T>>>,
    cursor: Option<DeqCursor<T>>,
    marker: PhantomData<Box<DeqNode<T>>>,
}

impl<T> Drop for Deque<T> {
    fn drop(&mut self) {
        struct DropGuard<'a, T>(&'a mut Deque<T>);

        impl<'a, T> Drop for DropGuard<'a, T> {
            fn drop(&mut self) {
                // Continue the same loop we do below. This only runs when a destructor has
                // panicked. If another one panics this will abort.
                while self.0.pop_front().is_some() {}
            }
        }

        while let Some(node) = self.pop_front() {
            let guard = DropGuard(self);
            drop(node);
            std::mem::forget(guard);
        }
    }
}

// Inner crate public function/methods
impl<T> Deque<T> {
    pub(crate) fn new(region: CacheRegion) -> Self {
        Self {
            region,
            len: 0,
            head: None,
            tail: None,
            cursor: None,
            marker: PhantomData::default(),
        }
    }

    pub(crate) fn region(&self) -> &CacheRegion {
        &self.region
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn contains(&self, node: &DeqNode<T>) -> bool {
        self.region == node.region && (node.prev.is_some() || self.is_head(node))
    }

    pub(crate) fn peek_front(&self) -> Option<&DeqNode<T>> {
        // This method takes care not to create mutable references to whole nodes,
        // to maintain validity of aliasing pointers into `element`.
        self.head.as_ref().map(|node| unsafe { node.as_ref() })
    }

    /// Removes and returns the node at the front of the list.
    pub(crate) fn pop_front(&mut self) -> Option<Box<DeqNode<T>>> {
        // This method takes care not to create mutable references to whole nodes,
        // to maintain validity of aliasing pointers into `element`.
        self.head.map(|node| unsafe {
            if self.is_at_cursor(node.as_ref()) {
                self.advance_cursor();
            }

            let mut node = Box::from_raw(node.as_ptr());
            self.head = node.next;

            match self.head {
                None => self.tail = None,
                // Not creating new mutable (unique!) references overlapping `element`.
                Some(head) => (*head.as_ptr()).prev = None,
            }

            self.len -= 1;

            node.prev = None;
            node.next = None;
            node
        })
    }

    #[cfg(test)]
    fn peek_back(&self) -> Option<&DeqNode<T>> {
        // This method takes care not to create mutable references to whole nodes,
        // to maintain validity of aliasing pointers into `element`.
        self.tail.as_ref().map(|node| unsafe { node.as_ref() })
    }

    /// Adds the given node to the back of the list.
    pub(crate) fn push_back(&mut self, mut node: Box<DeqNode<T>>) -> NonNull<DeqNode<T>> {
        // This method takes care not to create mutable references to whole nodes,
        // to maintain validity of aliasing pointers into `element`.
        unsafe {
            node.next = None;
            node.prev = self.tail;
            let node = NonNull::new(Box::into_raw(node)).expect("Got a null ptr");

            match self.tail {
                None => self.head = Some(node),
                // Not creating new mutable (unique!) references overlapping `element`.
                Some(tail) => (*tail.as_ptr()).next = Some(node),
            }

            self.tail = Some(node);
            self.len += 1;
            node
        }
    }

    pub(crate) unsafe fn move_to_back(&mut self, mut node: NonNull<DeqNode<T>>) {
        if self.is_tail(node.as_ref()) {
            // Already at the tail. Nothing to do.
            return;
        }

        if self.is_at_cursor(node.as_ref()) {
            self.advance_cursor();
        }

        let node = node.as_mut(); // this one is ours now, we can create an &mut.

        // Not creating new mutable (unique!) references overlapping `element`.
        match node.prev {
            Some(prev) if node.next.is_some() => (*prev.as_ptr()).next = node.next,
            Some(..) => (),
            // This node is the head node.
            None => self.head = node.next,
        };

        // This node is not the tail node.
        if let Some(next) = node.next.take() {
            (*next.as_ptr()).prev = node.prev;

            let mut node = NonNull::from(node);
            match self.tail {
                // Not creating new mutable (unique!) references overlapping `element`.
                Some(tail) => {
                    node.as_mut().prev = Some(tail);
                    (*tail.as_ptr()).next = Some(node)
                }
                None => unreachable!(),
            }
            self.tail = Some(node);
        }
    }

    /// Unlinks the specified node from the current list.
    ///
    /// This method takes care not to create mutable references to `element`, to
    /// maintain validity of aliasing pointers.
    ///
    /// Panics:
    pub(crate) unsafe fn unlink(&mut self, mut node: NonNull<DeqNode<T>>) {
        assert_eq!(self.region, node.as_ref().region);

        if self.is_at_cursor(node.as_ref()) {
            self.advance_cursor();
        }

        let node = node.as_mut(); // this one is ours now, we can create an &mut.

        // Not creating new mutable (unique!) references overlapping `element`.
        match node.prev {
            Some(prev) => (*prev.as_ptr()).next = node.next,
            // this node is the head node
            None => self.head = node.next,
        };

        match node.next {
            Some(next) => (*next.as_ptr()).prev = node.prev,
            // this node is the tail node
            None => self.tail = node.prev,
        };

        node.prev = None;
        node.next = None;

        self.len -= 1;
    }

    /// Unlinks the specified node from the current list, and then drop the node.
    ///
    /// This method takes care not to create mutable references to `element`, to
    /// maintain validity of aliasing pointers.
    ///
    /// Panics:
    pub(crate) unsafe fn unlink_and_drop(&mut self, node: NonNull<DeqNode<T>>) {
        self.unlink(node);
        std::mem::drop(Box::from_raw(node.as_ptr()));
    }

    #[allow(unused)]
    pub(crate) fn reset_cursor(&mut self) {
        self.cursor = None;
    }
}

impl<'a, T> Iterator for &'a mut Deque<T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor.is_none() {
            if let Some(head) = self.head {
                self.cursor = Some(DeqCursor::Node(head));
            }
        }
        let elem = if let Some(DeqCursor::Node(node)) = self.cursor {
            unsafe { Some(&(*node.as_ptr()).element) }
        } else {
            None
        };
        self.advance_cursor();
        elem
    }
}

// Private function/methods
impl<T> Deque<T> {
    fn is_head(&self, node: &DeqNode<T>) -> bool {
        if let Some(head) = self.head {
            std::ptr::eq(unsafe { head.as_ref() }, node)
        } else {
            false
        }
    }

    fn is_tail(&self, node: &DeqNode<T>) -> bool {
        if let Some(tail) = self.tail {
            std::ptr::eq(unsafe { tail.as_ref() }, node)
        } else {
            false
        }
    }

    fn is_at_cursor(&self, node: &DeqNode<T>) -> bool {
        if let Some(DeqCursor::Node(cur_node)) = self.cursor {
            std::ptr::eq(unsafe { cur_node.as_ref() }, node)
        } else {
            false
        }
    }

    fn advance_cursor(&mut self) {
        match self.cursor.take() {
            None => (),
            Some(DeqCursor::Node(node)) => unsafe {
                if let Some(next) = (*node.as_ptr()).next {
                    self.cursor = Some(DeqCursor::Node(next));
                } else {
                    self.cursor = Some(DeqCursor::Done);
                }
            },
            Some(DeqCursor::Done) => {
                self.cursor = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CacheRegion::MainProbation, DeqNode, Deque};

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn basics() {
        let mut deque: Deque<String> = Deque::new(MainProbation);
        assert_eq!(deque.len(), 0);
        assert!(deque.peek_front().is_none());
        assert!(deque.peek_back().is_none());

        // push_back(node1)
        let node1 = DeqNode::new(MainProbation, "a".to_string());
        assert!(!deque.contains(&node1));
        let node1 = Box::new(node1);
        let node1_ptr = deque.push_back(node1);
        assert_eq!(deque.len(), 1);

        // peek_front() -> node1
        let head_a = deque.peek_front().unwrap();
        assert!(deque.contains(&head_a));
        assert!(deque.is_head(&head_a));
        assert!(deque.is_tail(&head_a));
        assert_eq!(head_a.element, "a".to_string());

        // move_to_back(node1)
        unsafe { deque.move_to_back(node1_ptr.clone()) };
        assert_eq!(deque.len(), 1);

        // peek_front() -> node1
        let head_b = deque.peek_front().unwrap();
        assert!(deque.contains(&head_b));
        assert!(deque.is_head(&head_b));
        assert!(deque.is_tail(&head_b));
        assert!(std::ptr::eq(head_b, unsafe { node1_ptr.as_ref() }));
        assert!(head_b.prev.is_none());
        assert!(head_b.next.is_none());

        // peek_back() -> node1
        let tail_a = deque.peek_back().unwrap();
        assert!(deque.contains(&tail_a));
        assert!(deque.is_head(&tail_a));
        assert!(deque.is_tail(&tail_a));
        assert!(std::ptr::eq(tail_a, unsafe { node1_ptr.as_ref() }));
        assert!(tail_a.prev.is_none());
        assert!(tail_a.next.is_none());

        // push_back(node2)
        let node2 = DeqNode::new(MainProbation, "b".to_string());
        assert!(!deque.contains(&node2));
        let node2_ptr = deque.push_back(Box::new(node2));
        assert_eq!(deque.len(), 2);

        // peek_front() -> node1
        let head_c = deque.peek_front().unwrap();
        assert!(deque.contains(&head_c));
        assert!(deque.is_head(&head_c));
        assert!(!deque.is_tail(&head_c));
        assert!(std::ptr::eq(head_c, unsafe { node1_ptr.as_ref() }));
        assert!(head_c.prev.is_none());
        assert!(std::ptr::eq(
            unsafe { head_c.next.unwrap().as_ref() },
            unsafe { node2_ptr.as_ref() }
        ));

        // move_to_back(node2)
        unsafe { deque.move_to_back(node2_ptr.clone()) };
        assert_eq!(deque.len(), 2);

        // peek_front() -> node1
        let head_d = deque.peek_front().unwrap();
        assert!(deque.contains(&head_d));
        assert!(deque.is_head(&head_d));
        assert!(!deque.is_tail(&head_d));
        assert!(std::ptr::eq(head_d, unsafe { node1_ptr.as_ref() }));
        assert!(head_d.prev.is_none());
        assert!(std::ptr::eq(
            unsafe { head_d.next.unwrap().as_ref() },
            unsafe { node2_ptr.as_ref() }
        ));

        // peek_back() -> node2
        let tail_b = deque.peek_back().unwrap();
        assert!(deque.contains(&tail_b));
        assert!(!deque.is_head(&tail_b));
        assert!(deque.is_tail(&tail_b));
        assert!(std::ptr::eq(tail_b, unsafe { node2_ptr.as_ref() }));
        assert!(std::ptr::eq(
            unsafe { tail_b.prev.unwrap().as_ref() },
            unsafe { node1_ptr.as_ref() }
        ));
        assert_eq!(tail_b.element, "b".to_string());
        assert!(tail_b.next.is_none());

        // move_to_back(node1)
        unsafe { deque.move_to_back(node1_ptr.clone()) };
        assert_eq!(deque.len(), 2);

        // peek_front() -> node2
        let head_e = deque.peek_front().unwrap();
        assert!(deque.contains(&head_e));
        assert!(deque.is_head(&head_e));
        assert!(!deque.is_tail(&head_e));
        assert!(std::ptr::eq(head_e, unsafe { node2_ptr.as_ref() }));
        assert!(head_e.prev.is_none());
        assert!(std::ptr::eq(
            unsafe { head_e.next.unwrap().as_ref() },
            unsafe { node1_ptr.as_ref() }
        ));

        // peek_back() -> node1
        let tail_c = deque.peek_back().unwrap();
        assert!(deque.contains(&tail_c));
        assert!(!deque.is_head(&tail_c));
        assert!(deque.is_tail(&tail_c));
        assert!(std::ptr::eq(tail_c, unsafe { node1_ptr.as_ref() }));
        assert!(std::ptr::eq(
            unsafe { tail_c.prev.unwrap().as_ref() },
            unsafe { node2_ptr.as_ref() }
        ));
        assert!(tail_c.next.is_none());

        // push_back(node3)
        let node3 = DeqNode::new(MainProbation, "c".to_string());
        assert!(!deque.contains(&node3));
        let node3_ptr = deque.push_back(Box::new(node3));
        assert_eq!(deque.len(), 3);

        // peek_front() -> node2
        let head_f = deque.peek_front().unwrap();
        assert!(deque.contains(&head_f));
        assert!(deque.is_head(&head_f));
        assert!(!deque.is_tail(&head_f));
        assert!(std::ptr::eq(head_f, unsafe { node2_ptr.as_ref() }));
        assert!(head_f.prev.is_none());
        assert!(std::ptr::eq(
            unsafe { head_f.next.unwrap().as_ref() },
            unsafe { node1_ptr.as_ref() }
        ));

        // peek_back() -> node3
        let tail_d = deque.peek_back().unwrap();
        assert!(std::ptr::eq(tail_d, unsafe { node3_ptr.as_ref() }));
        assert_eq!(tail_d.element, "c".to_string());
        assert!(deque.contains(&tail_d));
        assert!(!deque.is_head(&tail_d));
        assert!(deque.is_tail(&tail_d));
        assert!(std::ptr::eq(tail_d, unsafe { node3_ptr.as_ref() }));
        assert!(std::ptr::eq(
            unsafe { tail_d.prev.unwrap().as_ref() },
            unsafe { node1_ptr.as_ref() }
        ));
        assert!(tail_d.next.is_none());

        // move_to_back(node1)
        unsafe { deque.move_to_back(node1_ptr.clone()) };
        assert_eq!(deque.len(), 3);

        // peek_front() -> node2
        let head_g = deque.peek_front().unwrap();
        assert!(deque.contains(&head_g));
        assert!(deque.is_head(&head_g));
        assert!(!deque.is_tail(&head_g));
        assert!(std::ptr::eq(head_g, unsafe { node2_ptr.as_ref() }));
        assert!(head_g.prev.is_none());
        assert!(std::ptr::eq(
            unsafe { head_g.next.unwrap().as_ref() },
            unsafe { node3_ptr.as_ref() }
        ));

        // peek_back() -> node1
        let tail_e = deque.peek_back().unwrap();
        assert!(deque.contains(&tail_e));
        assert!(!deque.is_head(&tail_e));
        assert!(deque.is_tail(&tail_e));
        assert!(std::ptr::eq(tail_e, unsafe { node1_ptr.as_ref() }));
        assert!(std::ptr::eq(
            unsafe { tail_e.prev.unwrap().as_ref() },
            unsafe { node3_ptr.as_ref() }
        ));
        assert!(tail_e.next.is_none());

        // unlink(node3)
        unsafe { deque.unlink(node3_ptr) };
        assert_eq!(deque.len(), 2);
        let node3_ref = unsafe { node3_ptr.as_ref() };
        assert!(!deque.contains(node3_ref));
        assert!(node3_ref.next.is_none());
        assert!(node3_ref.next.is_none());
        std::mem::drop(unsafe { Box::from_raw(node3_ptr.as_ptr()) });

        // peek_front() -> node2
        let head_h = deque.peek_front().unwrap();
        assert!(deque.contains(&head_h));
        assert!(deque.is_head(&head_h));
        assert!(!deque.is_tail(&head_h));
        assert!(std::ptr::eq(head_h, unsafe { node2_ptr.as_ref() }));
        assert!(head_h.prev.is_none());
        assert!(std::ptr::eq(
            unsafe { head_h.next.unwrap().as_ref() },
            unsafe { node1_ptr.as_ref() }
        ));

        // peek_back() -> node1
        let tail_f = deque.peek_back().unwrap();
        assert!(deque.contains(&tail_f));
        assert!(!deque.is_head(&tail_f));
        assert!(deque.is_tail(&tail_f));
        assert!(std::ptr::eq(tail_f, unsafe { node1_ptr.as_ref() }));
        assert!(std::ptr::eq(
            unsafe { tail_f.prev.unwrap().as_ref() },
            unsafe { node2_ptr.as_ref() }
        ));
        assert!(tail_f.next.is_none());

        // unlink(node2)
        unsafe { deque.unlink(node2_ptr) };
        assert_eq!(deque.len(), 1);
        let node2_ref = unsafe { node2_ptr.as_ref() };
        assert!(!deque.contains(node2_ref));
        assert!(node2_ref.next.is_none());
        assert!(node2_ref.next.is_none());
        std::mem::drop(unsafe { Box::from_raw(node2_ptr.as_ptr()) });

        // peek_front() -> node1
        let head_g = deque.peek_front().unwrap();
        assert!(deque.contains(&head_g));
        assert!(deque.is_head(&head_g));
        assert!(deque.is_tail(&head_g));
        assert!(std::ptr::eq(head_g, unsafe { node1_ptr.as_ref() }));
        assert!(head_g.prev.is_none());
        assert!(head_g.next.is_none());

        // peek_back() -> node1
        let tail_g = deque.peek_back().unwrap();
        assert!(deque.contains(&tail_g));
        assert!(deque.is_head(&tail_g));
        assert!(deque.is_tail(&tail_g));
        assert!(std::ptr::eq(tail_g, unsafe { node1_ptr.as_ref() }));
        assert!(tail_g.next.is_none());
        assert!(tail_g.next.is_none());

        // unlink(node1)
        unsafe { deque.unlink(node1_ptr) };
        assert_eq!(deque.len(), 0);
        let node1_ref = unsafe { node1_ptr.as_ref() };
        assert!(!deque.contains(node1_ref));
        assert!(node1_ref.next.is_none());
        assert!(node1_ref.next.is_none());
        std::mem::drop(unsafe { Box::from_raw(node1_ptr.as_ptr()) });

        // peek_front() -> node1
        let head_h = deque.peek_front();
        assert!(head_h.is_none());

        // peek_back() -> node1
        let tail_e = deque.peek_back();
        assert!(tail_e.is_none());
    }

    #[test]
    fn iter() {
        let mut deque: Deque<String> = Deque::new(MainProbation);
        assert!((&mut deque).next().is_none());

        let node1 = DeqNode::new(MainProbation, "a".into());
        deque.push_back(Box::new(node1));
        let node2 = DeqNode::new(MainProbation, "b".into());
        let node2_ptr = deque.push_back(Box::new(node2));
        let node3 = DeqNode::new(MainProbation, "c".into());
        let node3_ptr = deque.push_back(Box::new(node3));

        // -------------------------------------------------------
        // First iteration.
        assert_eq!((&mut deque).next(), Some(&"a".into()));
        assert_eq!((&mut deque).next(), Some(&"b".into()));
        assert_eq!((&mut deque).next(), Some(&"c".into()));
        assert!((&mut deque).next().is_none());

        // -------------------------------------------------------
        // Ensure the iterator restarts.
        assert_eq!((&mut deque).next(), Some(&"a".into()));
        assert_eq!((&mut deque).next(), Some(&"b".into()));
        assert_eq!((&mut deque).next(), Some(&"c".into()));
        assert!((&mut deque).next().is_none());

        // -------------------------------------------------------
        // Ensure reset_cursor works.
        assert_eq!((&mut deque).next(), Some(&"a".into()));
        assert_eq!((&mut deque).next(), Some(&"b".into()));
        deque.reset_cursor();
        assert_eq!((&mut deque).next(), Some(&"a".into()));
        assert_eq!((&mut deque).next(), Some(&"b".into()));
        assert_eq!((&mut deque).next(), Some(&"c".into()));
        assert!((&mut deque).next().is_none());

        // -------------------------------------------------------
        // Try to move_to_back during iteration.
        assert_eq!((&mut deque).next(), Some(&"a".into()));
        // Next will be "b", but we move it to the back.
        unsafe { deque.move_to_back(node2_ptr) };
        // Now, next should be "c", and then "b".
        assert_eq!((&mut deque).next(), Some(&"c".into()));
        assert_eq!((&mut deque).next(), Some(&"b".into()));
        assert!((&mut deque).next().is_none());

        // -------------------------------------------------------
        // Try to unlink during iteration.
        assert_eq!((&mut deque).next(), Some(&"a".into()));
        // Next will be "c", but we unlink it.
        unsafe { deque.unlink_and_drop(node3_ptr) };
        // Now, next should be "b".
        assert_eq!((&mut deque).next(), Some(&"b".into()));
        assert!((&mut deque).next().is_none());

        // -------------------------------------------------------
        // Try pop_front during iteration.
        let node3 = DeqNode::new(MainProbation, "c".into());
        deque.push_back(Box::new(node3));

        assert_eq!((&mut deque).next(), Some(&"a".into()));
        // Next will be "b", but we call pop_front twice to remove "a" and "b".
        deque.pop_front(); // "a"
        deque.pop_front(); // "b"
                           // Now, next should be "c".
        assert_eq!((&mut deque).next(), Some(&"c".into()));
        assert!((&mut deque).next().is_none());

        // -------------------------------------------------------
        // Check iterating on an empty deque.
        deque.pop_front(); // "c"
        assert!((&mut deque).next().is_none());
        assert!((&mut deque).next().is_none());
    }

    #[test]
    fn next_node() {
        let mut deque: Deque<String> = Deque::new(MainProbation);

        let node1 = DeqNode::new(MainProbation, "a".into());
        deque.push_back(Box::new(node1));
        let node2 = DeqNode::new(MainProbation, "b".into());
        let node2_ptr = deque.push_back(Box::new(node2));
        let node3 = DeqNode::new(MainProbation, "c".into());
        let node3_ptr = deque.push_back(Box::new(node3));

        // -------------------------------------------------------
        // First iteration.
        // peek_front() -> node1
        let node1a = deque.peek_front().unwrap();
        assert_eq!(node1a.element, "a".to_string());
        let node2a = node1a.next_node().unwrap();
        assert_eq!(node2a.element, "b".to_string());
        let node3a = node2a.next_node().unwrap();
        assert_eq!(node3a.element, "c".to_string());
        assert!(node3a.next_node().is_none());

        // -------------------------------------------------------
        // Iterate after a move_to_back.
        // Move "b" to the back. So now "a" -> "c" -> "b".
        unsafe { deque.move_to_back(node2_ptr) };
        let node1a = deque.peek_front().unwrap();
        assert_eq!(node1a.element, "a".to_string());
        let node3a = node1a.next_node().unwrap();
        assert_eq!(node3a.element, "c".to_string());
        let node2a = node3a.next_node().unwrap();
        assert_eq!(node2a.element, "b".to_string());
        assert!(node2a.next_node().is_none());

        // -------------------------------------------------------
        // Iterate after an unlink.
        // Unlink the second node "c". Now "a" -> "c".
        unsafe { deque.unlink_and_drop(node3_ptr) };
        let node1a = deque.peek_front().unwrap();
        assert_eq!(node1a.element, "a".to_string());
        let node2a = node1a.next_node().unwrap();
        assert_eq!(node2a.element, "b".to_string());
        assert!(node2a.next_node().is_none());
    }

    #[test]
    fn drop() {
        use std::{cell::RefCell, rc::Rc};

        struct X(u32, Rc<RefCell<Vec<u32>>>);

        impl Drop for X {
            fn drop(&mut self) {
                self.1.borrow_mut().push(self.0)
            }
        }

        let mut deque: Deque<X> = Deque::new(MainProbation);
        let dropped = Rc::new(RefCell::new(Vec::default()));

        let node1 = DeqNode::new(MainProbation, X(1, Rc::clone(&dropped)));
        let node2 = DeqNode::new(MainProbation, X(2, Rc::clone(&dropped)));
        let node3 = DeqNode::new(MainProbation, X(3, Rc::clone(&dropped)));
        let node4 = DeqNode::new(MainProbation, X(4, Rc::clone(&dropped)));
        deque.push_back(Box::new(node1));
        deque.push_back(Box::new(node2));
        deque.push_back(Box::new(node3));
        deque.push_back(Box::new(node4));
        assert_eq!(deque.len(), 4);

        std::mem::drop(deque);

        assert_eq!(*dropped.borrow(), &[1, 2, 3, 4]);
    }
}
