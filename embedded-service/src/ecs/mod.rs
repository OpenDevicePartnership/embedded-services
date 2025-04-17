//! Embedded Entity-Component System (ECS)
use core::{
    cell::{Ref, RefCell, RefMut},
    future::{self, Future},
    ops::{Deref, DerefMut},
};

use embassy_futures::select::{select, Either};

/// Used to chain the results of each layer together
pub enum List<A, B> {
    /// Event for the first layer
    Head(A),
    /// Events for the rest of the layers
    Rest(B),
}

// Type alias that may be helpful using layer results
/*pub type StackResult1<A> = List<A, ()>;
pub type StackResult2<A, B> = List<A, StackResult1<B>>;
pub type StackResult3<A, B, C> = List<A, StackResult2<B, C>>;*/

/// Trait to allow for borrowing a reference to the inner type
pub trait RefGuard<T>: Deref<Target = T> {}

/// Trait to allow for borrowing a mutable reference to the inner type
pub trait RefMutGuard<T>: DerefMut<Target = T> {}

/// Core component type
pub trait Component<T> {
    /// Event type that we're waiting for
    type Event;

    /// Wait for an event to occur
    fn wait_event(&self, entity: &T) -> impl Future<Output = Self::Event>;
    /// Process the event
    fn process(&self, entity: &mut T, event: Self::Event) -> impl Future<Output = ()>;
}

/// Entity trait
pub trait Entity {
    /// Underlying type of the entity
    type Inner;

    /// Get a reference to the inner entity
    fn get_entity(&self) -> impl RefGuard<Self::Inner>;
    /// Get a mutable reference to the inner entity
    fn get_entity_mut(&self) -> impl RefMutGuard<Self::Inner>;
}

/// Layer trait
pub trait Layer: Entity + Component<Self::Inner> + Sized {
    /// Process all events for the layer and layers below it
    fn process_all(&mut self) -> impl Future<Output = ()>;
}

/// Layer wrapper that provides common functionality and chaining logic
pub struct LayerWrapper<L: Layer, C: Component<L::Inner>> {
    inner: L,
    component: RefCell<C>,
}

impl<L: Layer, C: Component<L::Inner>> Entity for LayerWrapper<L, C> {
    type Inner = L::Inner;

    fn get_entity(&self) -> impl RefGuard<Self::Inner> {
        self.inner.get_entity()
    }

    fn get_entity_mut(&self) -> impl RefMutGuard<Self::Inner> {
        self.inner.get_entity_mut()
    }
}

impl<L: Layer, C: Component<L::Inner>> Component<L::Inner> for LayerWrapper<L, C> {
    type Event = List<C::Event, L::Event>;

    #[inline]
    async fn wait_event(&self, entity: &L::Inner) -> Self::Event {
        let mut borrow = self.component.borrow_mut();
        let component = borrow.deref_mut();

        match select(component.wait_event(entity), self.inner.wait_event(entity)).await {
            Either::First(event) => List::Head(event),
            Either::Second(event) => List::Rest(event),
        }
    }

    #[inline]
    async fn process(&self, entity: &mut L::Inner, event: Self::Event) -> () {
        let mut borrow = self.component.borrow_mut();
        let component = borrow.deref_mut();

        match event {
            List::Head(event) => component.process(entity, event).await,
            List::Rest(event) => self.inner.process(entity, event).await,
        }
    }
}

impl<L: Layer, C: Component<L::Inner>> Layer for LayerWrapper<L, C> {
    async fn process_all(&mut self) {
        let mut borrow = self.get_entity_mut();
        let entity = borrow.deref_mut();
        let event = self.wait_event(entity).await;
        self.process(entity, event).await;
    }
}

impl<L: Layer, C: Component<L::Inner>> LayerWrapper<L, C> {
    /// Create a new layer wrapper
    pub fn new(layer: L, component: C) -> Self {
        Self {
            inner: layer,
            component: RefCell::new(component),
        }
    }

    /// Add a new component as a new layer on top of us
    pub fn add_component<C2: Component<L::Inner>>(self, component: C2) -> LayerWrapper<Self, C2> {
        LayerWrapper::new(self, component)
    }
}

/// Entity that stores its value in a RefCell
pub struct EntityRef<T> {
    inner: RefCell<T>,
}

impl<T> RefGuard<T> for Ref<'_, T> {}
impl<T> RefMutGuard<T> for RefMut<'_, T> {}

impl<T> EntityRef<T> {
    /// Create a new entity reference
    pub fn new(entity: T) -> Self {
        Self {
            inner: RefCell::new(entity),
        }
    }

    /// Wrap the entity in a new component layer
    pub fn add_component<C: Component<T>>(self, component: C) -> LayerWrapper<Self, C> {
        LayerWrapper::new(self, component)
    }
}

impl<T> Entity for EntityRef<T> {
    type Inner = T;

    #[inline]
    fn get_entity(&self) -> impl RefGuard<Self::Inner> {
        self.inner.borrow()
    }

    #[inline]
    fn get_entity_mut(&self) -> impl RefMutGuard<Self::Inner> {
        self.inner.borrow_mut()
    }
}

impl<T> Component<T> for EntityRef<T> {
    type Event = ();

    #[inline]
    async fn wait_event(&self, _: &T) -> Self::Event {
        future::pending().await
    }

    #[inline]
    async fn process(&self, _: &mut T, _: Self::Event) {
        ()
    }
}

impl<T> Layer for EntityRef<T> {
    async fn process_all(&mut self) {
        ()
    }
}
