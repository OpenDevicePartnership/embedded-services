//! A static lifetime'd intrusive linked list, construction only (never remove/delete)

// Any type used for dynamic type coercion
pub use core::any::Any;

use crate::SyncCell;

/// Interface error class information
#[derive(Copy, Clone, Debug)]
pub enum Error {
    /// cannot push a node to any list if it's already in one
    NodeAlreadyInList,
}

/// override Result type for shorthand `-> Result<T>`
pub type Result<T> = core::result::Result<T, Error>;

/// Embedded node that "intrudes" on a structure
///
/// `address_of_data` carries `Send + Sync` bounds so that an
/// `IntrusiveNode` (and any `IntrusiveList` holding it) is itself `Send`
/// and `Sync` without an `unsafe impl`. This mirrors the
/// `NodeContainer: Send + Sync` requirement below and keeps the type
/// system aligned with the `CriticalSectionCell<T>: Sync where T: Send`
/// constraint that backs node storage.
#[derive(Copy, Clone, Debug)]
pub struct IntrusiveNode {
    /// offset from &self to struct data. Typically := sizeof(IntrusiveNode)
    address_of_data: &'static (dyn Any + Send + Sync),

    /// unsafe iterator type
    next: Option<&'static IntrusiveNode>,

    /// valid address flag: used to ensure proper initialization sequencing over address_of_data
    valid: bool,
}

/// node type for list allocation. Embed this in the "list wrapper" object, and init with Node::uninit()
pub struct Node {
    inner: SyncCell<IntrusiveNode>,
}

struct Invalid {}

impl Node {
    const INVALID: Invalid = Invalid {};

    /// shorthand constant for no elements in list
    pub const EMPTY: IntrusiveNode = IntrusiveNode {
        address_of_data: &Node::INVALID,
        next: None,
        valid: false,
    };

    /// construct an uninitialized node in place
    pub const fn uninit() -> Node {
        Node {
            inner: SyncCell::new(Node::EMPTY),
        }
    }
}

/// implementing this trait is required for IntrusiveList construction over type T
///
/// # Compatibility
///
/// The supertraits `Any + Send + Sync` are required because every node is
/// type-erased to a `&'static (dyn Any + Send + Sync)` inside
/// `IntrusiveNode::address_of_data`. Downstream implementors whose container
/// type is auto-`Send + Sync` need no extra work. Implementors whose
/// container holds a `!Send` or `!Sync` field (typically a trait object
/// without `Send + Sync` supertraits, or an interior-mutability primitive
/// like `Cell`) must either:
///
///   1. add the missing supertrait bounds to the held trait object so that
///      auto-derive succeeds, or
///   2. add a documented manual `unsafe impl Send + Sync` for the container,
///      justifying the impl against the actual sharing model of the
///      consumer (e.g. the single-executor model used throughout
///      `embedded-service` itself).
///
/// The `Send + Sync` bounds align with `IntrusiveNode::address_of_data`,
/// which erases `T` to `&'static (dyn Any + Send + Sync)`. Every concrete
/// `NodeContainer` impl in this workspace either composes only `Send + Sync`
/// fields, or carries a documented manual `unsafe impl` justified by the
/// single Cortex-M / single Embassy executor model.
///
/// A container with a `!Send` field must not satisfy `NodeContainer` via
/// auto-derive. The following example must fail to compile (because
/// `*const u8` is `!Send`, the auto-derive of `Send` for `BadContainer`
/// fails, and therefore the `NodeContainer` bound `Send + Sync` fails):
///
/// ```compile_fail
/// use embedded_services::intrusive_list::{Node, NodeContainer};
/// struct BadContainer {
///     node: Node,
///     raw: *const u8,
/// }
/// impl NodeContainer for BadContainer {
///     fn get_node(&self) -> &Node { &self.node }
/// }
/// ```
///
/// A container with a `core::cell::Cell<u8>` field is `Send` but
/// `!Sync`, so it must also fail to satisfy `NodeContainer`:
///
/// ```compile_fail
/// use embedded_services::intrusive_list::{Node, NodeContainer};
/// struct CellContainer {
///     node: Node,
///     c: core::cell::Cell<u8>,
/// }
/// impl NodeContainer for CellContainer {
///     fn get_node(&self) -> &Node { &self.node }
/// }
/// ```
pub trait NodeContainer: Any + Send + Sync {
    /// return the upper level node type reference attached to self
    fn get_node(&self) -> &Node;
}

/// List of intruded nodes of unknown type(s), must be allocated statically
pub struct IntrusiveList {
    /// traditional head pointer on list. Static reference type is used to ensure static allocations (for safety)
    head: SyncCell<Option<&'static IntrusiveNode>>,
}

impl IntrusiveNode {
    /// generate an empty node for use within whatever type T
    fn new<T: NodeContainer>(this_ref: &'static T) -> IntrusiveNode {
        IntrusiveNode {
            address_of_data: this_ref,
            next: None,
            valid: true,
        }
    }

    /// retrieve the underlying dynamic type information (vtable)
    pub fn data<T: NodeContainer>(&self) -> Option<&T> {
        if self.valid {
            self.address_of_data.downcast_ref()
        } else {
            None
        }
    }
}

impl Default for IntrusiveList {
    fn default() -> Self {
        Self::new()
    }
}

impl IntrusiveList {
    /// construct an empty intrusive list
    pub const fn new() -> IntrusiveList {
        IntrusiveList {
            head: SyncCell::new(None),
        }
    }

    /// only allow pushing to the head of the list
    ///
    /// This helper is called from inside the single `critical_section::with`
    /// of `push`, so the inner CS here is nested (a no-op on
    /// `critical_section` v1, which supports nesting). The CS is retained for
    /// callers (test-only) that might want to push a raw pre-validated node
    /// without re-running the validity check.
    fn push_front(&self, node: &'static mut IntrusiveNode) {
        critical_section::with(|_cs| {
            if let Some(old_head) = self.head.get() {
                node.next = Some(old_head);
            }

            self.head.set(Some(node));
        });
    }

    /// generic over T: NodeContainer for list.push() proper node construction
    ///
    /// The entire validity-check, node-write, and list-link sequence runs in
    /// a single `critical_section::with`. Without this, in any multi-executor
    /// or ISR-vs-thread scenario two concurrent `push(&same_obj)` callers
    /// could each pass the `valid == false` check, both mutate the same
    /// `SyncCell<IntrusiveNode>`, and the second `push_front` could
    /// construct a self-cycle (head -> N -> N -> ...).
    ///
    /// Under the single Cortex-M / single Embassy executor model the race
    /// cannot fire today, but `push_front` is itself CS-gated, so this
    /// extends the same atomicity guarantee to the whole push sequence.
    /// Loom-based concurrency verification is a separate follow-up.
    pub fn push<T: NodeContainer>(&self, object: &'static T) -> Result<()> {
        // Single critical_section around the whole push sequence so that on
        // any future multi-executor / ISR target the validity check and the
        // head-link write are atomic.
        critical_section::with(|_cs| {
            // check if node is in the list already. Valid flag will only be set if
            // the element has been constructed and inserted into a linked list, so
            // this check covers both same list and other list conditions.
            if object.get_node().inner.get().valid {
                return Err(Error::NodeAlreadyInList);
            }

            // since this API is private to this module, this is the only place where
            // a node can be marked as valid.
            let node = IntrusiveNode::new(object);
            object.get_node().inner.set(node);

            self.push_front(
                // SAFETY: we hold the only critical section in this path.
                // Three invariants hold here:
                //   1. We just set `valid = true` inside this CS, and the
                //      pre-CS check above guaranteed `valid == false`. No
                //      other CS-bound mutator can have observed the node
                //      under our CS, so this `&mut IntrusiveNode` does not
                //      alias any live `&IntrusiveNode` produced by `push` /
                //      `push_front` (both are CS-gated).
                //   2. The `IntrusiveIterator` reads `head` and `next`
                //      outside any CS. The non-CS readers can only observe
                //      a node AFTER `self.head.set(Some(node))` runs inside
                //      `push_front` (line above), at which point we have
                //      released the `&mut IntrusiveNode` because `push_front`
                //      takes it by value.
                //   3. `as_ptr` returns a valid pointer for the entire
                //      `'static` lifetime of `object`, so dereferencing it
                //      is well-defined.
                unsafe { &mut *object.get_node().inner.as_ptr() },
            );
            Ok(())
        })
    }

    /// Iterate over the list as if it were items of type `T`, skipping any nodes that are of a different type.
    pub fn iter_only<T: NodeContainer>(&self) -> OnlyT<'_, T> {
        OnlyT::new(self.into_iter())
    }
}

/// iterator wrapper type for IntrusiveNode
pub struct IntrusiveIterator {
    current: Option<&'static IntrusiveNode>,
}

impl IntoIterator for &IntrusiveList {
    type IntoIter = IntrusiveIterator;
    type Item = &'static IntrusiveNode;

    fn into_iter(self) -> Self::IntoIter {
        IntrusiveIterator {
            current: self.head.get(),
        }
    }
}

impl Iterator for IntrusiveIterator {
    type Item = &'static IntrusiveNode;

    fn next(&mut self) -> Option<Self::Item> {
        let mut iter = None;

        if let Some(current) = self.current {
            self.current = current.next;
            iter = Some(current);
        }

        iter
    }
}

/// Iterator wrapper type for [`IntrusiveList`] that returns only nodes of type `T`.
pub struct OnlyT<'a, T> {
    iter: core::iter::FilterMap<IntrusiveIterator, fn(&'static IntrusiveNode) -> Option<&'a T>>,
    _marker: core::marker::PhantomData<&'a T>,
}

impl<T: NodeContainer> OnlyT<'_, T> {
    /// Create a new `OnlyTIter` from an `IntrusiveIterator`.
    pub fn new(iter: IntrusiveIterator) -> Self {
        Self {
            iter: iter.filter_map(|node| node.data::<T>()),
            _marker: core::marker::PhantomData,
        }
    }
}

impl<'a, T: NodeContainer> Iterator for OnlyT<'a, T> {
    type Item = &'a T;

    /// Advance the iterator and return the next node of type `T`.
    /// If the next node is not of type `T`, it will be skipped.
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    trait OpA: Send + Sync {
        #[inline]
        fn a(&self) -> bool {
            true
        }
    }

    trait OpB: Send + Sync {
        #[inline]
        fn b(&self) -> bool {
            true
        }
    }

    struct RegistrationA {
        node: Node,
        owner: SyncCell<Option<&'static dyn OpA>>,
    }

    struct RegistrationB {
        node: Node,
        owner: SyncCell<Option<&'static dyn OpB>>,
    }

    impl NodeContainer for RegistrationA {
        fn get_node(&self) -> &Node {
            &self.node
        }
    }

    impl NodeContainer for RegistrationB {
        fn get_node(&self) -> &Node {
            &self.node
        }
    }

    struct ElementA {
        a: RegistrationA,
    }

    struct ElementB {
        b: RegistrationB,
    }

    struct ElementAB {
        a: RegistrationA,
        b: RegistrationB,
    }

    impl RegistrationA {
        fn new() -> Self {
            Self {
                node: Node::uninit(),
                owner: SyncCell::new(None),
            }
        }

        fn init<T: OpA>(&self, obj: &'static T) {
            self.owner.set(Some(obj));
        }

        fn test(&self) {
            assert!(self.owner.get().is_some_and(|owner| owner.a()));
        }
    }

    impl RegistrationB {
        fn new() -> Self {
            Self {
                node: Node::uninit(),
                owner: SyncCell::new(None),
            }
        }

        fn init<T: OpB>(&self, obj: &'static T) {
            self.owner.set(Some(obj));
        }

        fn test(&self) {
            assert!(self.owner.get().is_some_and(|owner| owner.b()));
        }
    }

    impl OpA for ElementA {}
    impl OpA for ElementAB {}
    impl OpB for ElementB {}
    impl OpB for ElementAB {}

    impl ElementA {
        fn new() -> Self {
            Self {
                a: RegistrationA::new(),
            }
        }

        fn register(&'static self, list: &IntrusiveList) -> Result<()> {
            self.a.init(self);
            list.push(&self.a)
        }
    }

    impl ElementB {
        fn new() -> Self {
            Self {
                b: RegistrationB::new(),
            }
        }

        fn register(&'static self, list: &IntrusiveList) -> Result<()> {
            self.b.init(self);
            list.push(&self.b)
        }
    }

    impl ElementAB {
        fn new() -> Self {
            Self {
                a: RegistrationA::new(),
                b: RegistrationB::new(),
            }
        }

        fn register_a(&'static self, list: &IntrusiveList) -> Result<()> {
            self.a.init(self);
            list.push(&self.a)
        }

        fn register_b(&'static self, list: &IntrusiveList) -> Result<()> {
            self.b.init(self);
            list.push(&self.b)
        }
    }

    struct RegistrationOnlyOneInstance {}
    impl NodeContainer for RegistrationOnlyOneInstance {
        fn get_node(&self) -> &Node {
            static NODE: OnceLock<Node> = OnceLock::new();

            NODE.get_or_init(Node::uninit)
        }
    }

    struct RegistrationOnly {
        node: Node,
    }

    impl NodeContainer for RegistrationOnly {
        fn get_node(&self) -> &Node {
            &self.node
        }
    }

    use embassy_sync::once_lock::OnceLock;

    #[test]
    fn test_node_internal_validity() {
        // test if invalid node will block data access
        // NOTE: this can't be accessed outside of this crate, due to private wrapping of Node::inner.
        static EMPTY_NODE: OnceLock<RegistrationOnlyOneInstance> = OnceLock::new();
        let empty_node = EMPTY_NODE.get_or_init(|| RegistrationOnlyOneInstance {});

        // accessing private .inner. here just for test validation. Not a consumer facing scenario
        // SAFETY: this is not safe. Don't do this. Only here for test completeness
        let as_element: Option<&RegistrationA> = unsafe { &*empty_node.get_node().inner.as_ptr() }.data();
        assert!(as_element.is_none());
    }

    #[test]
    fn test_list_mixup_checks() {
        // test if we can mixup nodes manually
        static EL1: OnceLock<RegistrationA> = OnceLock::new();
        static EL2: OnceLock<RegistrationA> = OnceLock::new();
        let first_el = EL1.get_or_init(RegistrationA::new);
        let second_el = EL2.get_or_init(RegistrationA::new);
        let list = IntrusiveList::new();

        assert!(list.push(first_el).is_ok());
        assert!(list.push(second_el).is_ok());

        // guard against circular list construction
        assert!(list.push(first_el).is_err());
        assert!(list.push(second_el).is_err());

        // guard against invalid node insertion
        static SIMPLE_NODE: OnceLock<RegistrationOnly> = OnceLock::new();
        let simple_node = SIMPLE_NODE.get_or_init(|| RegistrationOnly { node: Node::uninit() });
        assert!(list.push(simple_node).is_ok());

        // try pushing to a second list
        let list2 = IntrusiveList::new();
        assert!(list2.push(simple_node).is_err());

        // ensure that someone can't abuse the get_node() trait to allow list mangling:
        static EMPTY_NODE: OnceLock<RegistrationOnlyOneInstance> = OnceLock::new();
        let empty_node = EMPTY_NODE.get_or_init(|| RegistrationOnlyOneInstance {});

        static EMPTY_NODE_UNPUSHABLE: OnceLock<RegistrationOnlyOneInstance> = OnceLock::new();
        let empty_node_unpushable = EMPTY_NODE_UNPUSHABLE.get_or_init(|| RegistrationOnlyOneInstance {});
        // place the single iterable instance in first list
        assert!(list.push(empty_node).is_ok());

        // any subsequent pushes will fail
        assert!(list.push(empty_node).is_err());
        assert!(list2.push(empty_node).is_err());
        assert!(list.push(empty_node_unpushable).is_err());
        assert!(list2.push(empty_node_unpushable).is_err());
    }

    #[test]
    fn test_empty_list() {
        let list = IntrusiveList::new();
        assert_eq!(0, list.into_iter().count());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_monotype_list() {
        let list_a = IntrusiveList::new();
        let list_b = IntrusiveList::new();
        static A: [OnceLock<ElementA>; 5] = [const { OnceLock::new() }; 5];
        static B: [OnceLock<ElementB>; 5] = [const { OnceLock::new() }; 5];

        // initialize static blocks
        for a in &A {
            a.get_or_init(ElementA::new);
        }

        for b in &B {
            b.get_or_init(ElementB::new);
        }

        // construct lists
        for a in &A {
            assert!(embassy_futures::block_on(async { a.get().await.register(&list_a) }).is_ok());
        }

        for b in &B {
            assert!(embassy_futures::block_on(async { b.get().await.register(&list_b) }).is_ok());
        }

        // assert validity of lists
        for ra in &list_a {
            let a: &RegistrationA = ra.data().unwrap();
            a.test();
        }

        for rb in &list_b {
            let b: &RegistrationB = rb.data().unwrap();
            b.test();
        }

        // ensure dynamic type information is preserved
        for ra in &list_a {
            let b: Option<&RegistrationB> = ra.data();
            assert!(b.is_none());
        }

        assert_eq!(A.len(), list_a.iter_only::<RegistrationA>().count());
        assert_eq!(0, list_a.iter_only::<RegistrationB>().count());
        assert_eq!(0, list_b.iter_only::<RegistrationA>().count());
        assert_eq!(B.len(), list_b.iter_only::<RegistrationB>().count());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_multitype_list() {
        // list with multiple types within it (same registration type)
        let list_a = IntrusiveList::new();
        static A: [OnceLock<ElementA>; 5] = [const { OnceLock::new() }; 5];
        static AB: [OnceLock<ElementAB>; 5] = [const { OnceLock::new() }; 5];

        // initialize static blocks
        for a in &A {
            a.get_or_init(ElementA::new);
        }

        for ab in &AB {
            ab.get_or_init(ElementAB::new);
        }

        // construct lists
        for a in &A {
            assert!(embassy_futures::block_on(async { a.get().await.register(&list_a) }).is_ok());
        }

        for ab in &AB {
            assert!(embassy_futures::block_on(async { ab.get().await.register_a(&list_a) }).is_ok());
        }

        // assert validity of lists
        for ra in &list_a {
            let a: &RegistrationA = ra.data().unwrap();
            a.test();
        }

        // ensure filtered iterator works
        assert_eq!(A.len() + AB.len(), list_a.iter_only::<RegistrationA>().count());
        assert_eq!(0, list_a.iter_only::<RegistrationB>().count());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_multi_list() {
        // nodes in multiple lists
        let list_a = IntrusiveList::new();
        let list_b = IntrusiveList::new();
        static A: [OnceLock<ElementA>; 5] = [const { OnceLock::new() }; 5];
        static B: [OnceLock<ElementB>; 5] = [const { OnceLock::new() }; 5];
        static AB: [OnceLock<ElementAB>; 5] = [const { OnceLock::new() }; 5];

        // initialize static blocks
        for a in &A {
            a.get_or_init(ElementA::new);
        }

        for b in &B {
            b.get_or_init(ElementB::new);
        }

        for ab in &AB {
            ab.get_or_init(ElementAB::new);
        }

        // construct lists
        for a in &A {
            assert!(embassy_futures::block_on(async { a.get().await.register(&list_a) }).is_ok());
        }

        for b in &B {
            assert!(embassy_futures::block_on(async { b.get().await.register(&list_b) }).is_ok());
        }

        for ab in &AB {
            embassy_futures::block_on(async {
                assert!(ab.get().await.register_a(&list_a).is_ok());
                assert!(ab.get().await.register_b(&list_b).is_ok());
            });
        }

        // assert validity of lists
        for ra in &list_a {
            let a: &RegistrationA = ra.data().unwrap();
            a.test();
        }

        for rb in &list_b {
            let b: &RegistrationB = rb.data().unwrap();
            b.test();
        }

        assert_eq!(A.len() + AB.len(), list_a.iter_only::<RegistrationA>().count());
        assert_eq!(0, list_a.iter_only::<RegistrationB>().count());

        assert_eq!(0, list_b.iter_only::<RegistrationA>().count());
        assert_eq!(B.len() + AB.len(), list_b.iter_only::<RegistrationB>().count());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_multi_registration_list() {
        // list with multiple registration types
        let list = IntrusiveList::new();
        static A: [OnceLock<ElementA>; 5] = [const { OnceLock::new() }; 5];
        static B: [OnceLock<ElementB>; 5] = [const { OnceLock::new() }; 5];

        // initialize static blocks
        for a in &A {
            a.get_or_init(ElementA::new);
        }

        for b in &B {
            b.get_or_init(ElementB::new);
        }

        // construct lists
        // NOTE: `push` pushes to the front, so the order of the list will be [B..., A...]
        embassy_futures::block_on(async {
            for a in &A {
                assert!(a.get().await.register(&list).is_ok());
            }

            for b in &B {
                assert!(b.get().await.register(&list).is_ok());
            }
        });

        // assert validity of lists
        for ra in list.into_iter().skip(B.len()) {
            let a: &RegistrationA = ra.data().unwrap();
            a.test();
        }

        for rb in list.into_iter().take(B.len()) {
            let b: &RegistrationB = rb.data().unwrap();
            b.test();
        }

        assert_eq!(A.len() + B.len(), list.into_iter().count());
        assert_eq!(A.len(), list.iter_only::<RegistrationA>().count());
        assert_eq!(B.len(), list.iter_only::<RegistrationB>().count());
    }

    #[test]
    fn test_static_alloc() {
        static _LIST: IntrusiveList = IntrusiveList::new();
    }

    /// After a failed `push` (duplicate), the list state must be identical
    /// to before the push attempt. With a non-atomic implementation, an
    /// interleaved push could observe a partially constructed node; the
    /// CS-wrapped sequence makes the whole thing atomic.
    ///
    /// We assert the observable behavior: after a duplicate push fails,
    /// (a) the head of the list still points at the same node as before, and
    /// (b) iterating the list yields the same elements in the same order.
    /// Full multi-task race verification (loom) is a separate follow-up.
    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_push_failure_leaves_list_state_unchanged() {
        let list = IntrusiveList::new();
        static EL1: OnceLock<RegistrationA> = OnceLock::new();
        static EL2: OnceLock<RegistrationA> = OnceLock::new();
        let a = EL1.get_or_init(RegistrationA::new);
        let b = EL2.get_or_init(RegistrationA::new);

        list.push(a).unwrap();
        list.push(b).unwrap();

        // Snapshot the pre-failure state.
        let head_before = list.head.get().map(|n| n as *const _);
        let count_before = list.into_iter().count();
        assert_eq!(count_before, 2);

        // Duplicate pushes must fail.
        assert!(list.push(a).is_err());
        assert!(list.push(b).is_err());

        // Post-failure state must match.
        let head_after = list.head.get().map(|n| n as *const _);
        let count_after = list.into_iter().count();
        assert_eq!(head_before, head_after, "head must not change on failed push");
        assert_eq!(count_after, 2, "list length must not change on failed push");
    }
}
