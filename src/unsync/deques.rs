use super::{KeyDate, KeyHashDate, ValueEntry};
use crate::common::deque::{CacheRegion, DeqNode, Deque};

use std::ptr::NonNull;

pub(crate) struct Deques<K> {
    pub(crate) window: Deque<KeyHashDate<K>>, //    Not used yet.
    pub(crate) probation: Deque<KeyHashDate<K>>,
    pub(crate) protected: Deque<KeyHashDate<K>>, // Not used yet.
    pub(crate) write_order: Deque<KeyDate<K>>,
}

impl<K> Default for Deques<K> {
    fn default() -> Self {
        Self {
            window: Deque::new(CacheRegion::Window),
            probation: Deque::new(CacheRegion::MainProbation),
            protected: Deque::new(CacheRegion::MainProtected),
            write_order: Deque::new(CacheRegion::WriteOrder),
        }
    }
}

impl<K> Deques<K> {
    pub(crate) fn clear(&mut self) {
        self.window = Deque::new(CacheRegion::Window);
        self.probation = Deque::new(CacheRegion::MainProbation);
        self.protected = Deque::new(CacheRegion::MainProtected);
        self.write_order = Deque::new(CacheRegion::WriteOrder);
    }

    pub(crate) fn push_back_ao<V>(
        &mut self,
        region: CacheRegion,
        kh: KeyHashDate<K>,
        entry: &mut ValueEntry<K, V>,
    ) {
        use CacheRegion::*;
        let node = Box::new(DeqNode::new(region, kh));
        let node = match node.as_ref().region {
            Window => self.window.push_back(node),
            MainProbation => self.probation.push_back(node),
            MainProtected => self.protected.push_back(node),
            WriteOrder => unreachable!(),
        };
        entry.set_access_order_q_node(Some(node));
    }

    pub(crate) fn push_back_wo<V>(&mut self, kh: KeyDate<K>, entry: &mut ValueEntry<K, V>) {
        let node = Box::new(DeqNode::new(CacheRegion::WriteOrder, kh));
        let node = self.write_order.push_back(node);
        entry.set_write_order_q_node(Some(node));
    }

    pub(crate) fn move_to_back_ao<V>(&mut self, entry: &ValueEntry<K, V>) {
        use CacheRegion::*;
        let node = entry.access_order_q_node().unwrap();
        let p = unsafe { node.as_ref() };
        match &p.region {
            Window if self.window.contains(p) => unsafe { self.window.move_to_back(node) },
            MainProbation if self.probation.contains(p) => unsafe {
                self.probation.move_to_back(node)
            },
            MainProtected if self.protected.contains(p) => unsafe {
                self.protected.move_to_back(node)
            },
            _ => {}
        }
    }

    pub(crate) fn move_to_back_wo<V>(&mut self, entry: &ValueEntry<K, V>) {
        use CacheRegion::*;
        let node = entry.write_order_q_node().unwrap();
        let p = unsafe { node.as_ref() };
        debug_assert_eq!(&p.region, &WriteOrder);
        if self.write_order.contains(p) {
            unsafe { self.write_order.move_to_back(node) };
        }
    }

    pub(crate) fn unlink_ao<V>(&mut self, entry: &mut ValueEntry<K, V>) {
        if let Some(node) = entry.take_access_order_q_node() {
            self.unlink_node_ao(node);
        }
    }

    pub(crate) fn unlink_ao_from_deque<V>(
        deq_name: &str,
        deq: &mut Deque<KeyHashDate<K>>,
        entry: &mut ValueEntry<K, V>,
    ) {
        if let Some(node) = entry.take_access_order_q_node() {
            unsafe { Self::unlink_node_ao_from_deque(deq_name, deq, node) };
        }
    }

    pub(crate) fn unlink_wo<V>(deq: &mut Deque<KeyDate<K>>, entry: &mut ValueEntry<K, V>) {
        if let Some(node) = entry.take_write_order_q_node() {
            Self::unlink_node_wo(deq, node);
        }
    }

    pub(crate) fn unlink_node_ao(&mut self, node: NonNull<DeqNode<KeyHashDate<K>>>) {
        use CacheRegion::*;
        unsafe {
            match node.as_ref().region {
                Window => Self::unlink_node_ao_from_deque("window", &mut self.window, node),
                MainProbation => {
                    Self::unlink_node_ao_from_deque("probation", &mut self.probation, node)
                }
                MainProtected => {
                    Self::unlink_node_ao_from_deque("protected", &mut self.protected, node)
                }
                _ => unreachable!(),
            }
        }
    }

    unsafe fn unlink_node_ao_from_deque(
        deq_name: &str,
        deq: &mut Deque<KeyHashDate<K>>,
        node: NonNull<DeqNode<KeyHashDate<K>>>,
    ) {
        if deq.contains(node.as_ref()) {
            // https://github.com/moka-rs/moka/issues/64
            deq.unlink_and_drop(node);
        } else {
            panic!(
                "unlink_node - node is not a member of {} deque. {:?}",
                deq_name,
                node.as_ref()
            )
        }
    }

    pub(crate) fn unlink_node_wo(deq: &mut Deque<KeyDate<K>>, node: NonNull<DeqNode<KeyDate<K>>>) {
        use CacheRegion::*;
        unsafe {
            let p = node.as_ref();
            debug_assert_eq!(&p.region, &WriteOrder);
            if deq.contains(p) {
                // https://github.com/moka-rs/moka/issues/64
                deq.unlink_and_drop(node);
            } else {
                panic!(
                    "unlink_node - node is not a member of write_order deque. {:?}",
                    p
                )
            }
        }
    }
}
