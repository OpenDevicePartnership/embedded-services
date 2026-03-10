# Service API Guidelines

This document establishes some guidelines that APIs in this repo should try to conform to, and explains the rationale behind those guidelines to help guide decisionmaking when tradeoffs need to be made.

These guidelines attempt to make our APIs easier to compose, test, and use.

## Guidelines

### No 'static references

References with lifetime `'static` in API functions should be avoided, even if the lifetime will always be `'static` in production use cases. Instead, make your type generic over a lifetime with the expectation that that lifetime will be `'static` in production use cases.

__Reason__: Testability. If something needs to take a reference to an object 'O' with lifetime `'static`, that means that O can never be destroyed.  This can make it pretty difficult to test things that use that API.  

__Example__:
Instead of this:
```rust
trait Subscriber {}
struct Notifier { subscriber: &'static dyn Subscriber }
//                            ^^^^^^^^

impl Notifier {
    fn new(subscriber: &'static dyn Subscriber) -> Self {
        //             ^^^^^^^^
        Self { subscriber }
    }
}
```

Consider something like this:
```rust
trait Subscriber {}
struct Notifier<'sub> { subscriber: &'sub dyn Subscriber }
//             ^^^^^^               ^^^^^

impl<'sub> Notifier<'sub> {
    fn new(subscriber: &'sub dyn Subscriber) -> Self {
        //             ^^^^^
        Self { subscriber }
    }
}
```
In cases like this, if you know that there will only be one concrete type for your reference, consider being generic over the type rather than taking it as `dyn`. This is particularly common for HAL trait implementations.  This allows the compiler to inline and simplify code, which can result in performance and code size improvements in some circumstances.

Alternatively, if you can take an owned `Subscriber` rather than a reference, something like this is probably better:
```rust
trait Subscriber {}
struct Notifier<S> {
    sub: S,
}

impl<S: Subscriber> Notifier<S> {
    const fn new(sub: S) -> Self {
        Self { sub }
    }
}
```

### External memory allocation / no static memory allocation

Memory allocation should always be the role of the caller of the API.  If you need memory, have your caller pass it into your constructor.  Do not have things like `static INSTANCE: OnceLock<MyService>` in your service module.

If you don't need dynamic dispatch over user-provided types, additionally consider being generic over those user-provided types rather than taking `dyn` arguments - this is only possible if you have external memory allocation.

Note that while this applies to code in this repo, it does not necessarily apply to other ODP repos (e.g. HAL crates that know exactly how many instances of peripheral X are available on the platform).

__Reason__: Most code in this repo is expected to run primarily in environments that don't have a heap.  In heapless environments, your options are either to have your caller provide you memory or to allocate it as a static variable in your module.  Allocating it as a static variable in your module has negative impacts on flexibility, testability, performance, and code size.
Flexibility - Memory allocation in your module rather than by your caller means that the size of your object must be known when the module is compiled rather than when you're instantiated. This prevents you from storing any owned caller-provided types in your object (since you can't know those types when your module is compiled).
Testability - if you have a private singleton instance, tests can't arbitrarily destroy and recreate that state.  This makes it difficult to test multiple startup paths.
Performance - if you can't be generic over a type, the only way you can interact with user-provided types is by dyn references to trait impls.  External memory allocation allows you to be generic over a type, which means you don't have to pay for dynamic dispatch and the compiler can potentially inline code / optimize the interaction between your code and the user-provided type's code.
Code size - The compiler has to generate a bunch of code to handle dynamic dispatch, even if there's only ever a single concrete type that implements the trait you want to be generic over, which is common with HAL traits.

__Example__:
Note that in the below example, the `OnceLock` / external `Resources` is only necessary if you need to hand out references to the contents of the `OnceLock` / `FooInner`. That's elided in the example and assumed to be implemented in the `/* .. */` blocks for simplicity.
```rust
pub struct Foo { /* .. */ }

static INSTANCE: OnceLock<Foo> = OnceLock::new();

impl Foo {
    async fn init(/* .. */) -> &'static Foo {
        let instance = INSTANCE.get_or_init(|| Foo{ /* .. */ }).await;

        // Create another reference to some state in 'inner' - perhaps by passing it to something in /* .. */

        instance
    }
}
```

Consider something like this:
```rust
struct FooInner<'hw> { /* .. */ }

#[derive(Default)]
pub struct Resources<'hw> {
    inner: Option<FooInner>
}

pub struct Foo<'hw> {
    inner: &'hw FooInner
}

impl<'hw> Foo<'hw> {
    fn new(resources: &'hw mut Resources, /* .. */) -> Self {
        let inner = resources.insert(FooInner::new(/* .. */));

        // Create another reference to some state in `inner` here that outlasts this function - perhaps by returning
        // a `Runner` that contains a reference to `inner` or passing a reference to `inner` to one of the elided
        // arguments in /* .. */.  See the 'Use runner objects for concurrency' section for a concrete example of this.
        // If you don't have a requirement to do this, you don't need the indirection / external `Resources` object at all.

        Self{ inner }
    }
}
```


### Use runner objects for concurrency

Don't declare embassy tasks in your module - instead, have the constructor for your type return a `(Self, Runner)` tuple. The `Runner` object should have a single method `run(self) -> !` that the entity that instantiated your object must execute.  You should have only one `Runner` object returned. Use the `odp-service-common::runnable_service::Service` trait to enforce this pattern.

__Reason__: Declarations of embassy tasks are functionally static memory allocations. They can't be generic and you have to declare at declaration time a maximum number of instances that can be run concurrently. They also commit you to running on embassy, which is not necessarily desirable in test contexts.  Pushing responsibility for the allocation out of your module allows your types to be generic.  
However, it also means that your caller needs to be able to declare a task that can run your runner, and if you have multiple things that each need different pieces of state and need to run concurrently, setting up those tasks can make your API unwieldy and brittle.
Returning a simple `Runner` object at the same time as your object makes it difficult to forget to execute the runner.
Allowing only a single `Runner` with only one method that takes no external arguments makes it difficult to misuse the runner.

__Example__:
Instead of this:
```rust
///// Your type's definition /////
struct MyRunnableTypeInner { /* .. */ }

impl<'hw> MyRunnableTypeInner<'hw> {
    /* .. */
}

#[derive(Default)]
pub struct Resources<'hw> {
    inner: Option<MyRunnableTypeInner>
}

pub struct MyRunnableType<'hw> {
    inner: &'hw MyRunnableTypeInner
}

impl<'hw> MyRunnableType<'hw> {
    pub fn new(resources: &mut Resources, /* .. */ ) -> Self {
        let inner = resources.insert(RunnableTypeInner::new(/* .. */))
        /* .. */ 
        Self { inner }
    }
}

mod tasks {
    pub async fn run_task_1(runnable: &MyRunnableType, foo: Foo) -> ! { /* .. */ }
    pub async fn run_task_2(runnable: &MyRunnableType, bar: Bar) -> ! { /* .. */ }
    pub async fn run_task_3(runnable: &MyRunnableType, baz: Baz) -> ! { /* .. */ }
}

///// End-user code /////

fn main() {
    let instance = MyRunnableType::new(/* .. */);
    #[embassy_task]
    fn runner_1(runnable: &'static MyRunnableType, foo: Foo) -> ! {
        my_runnable_type::tasks::run_task_1(runnable, foo).await
    }
    #[embassy_task]
    fn runner_2(runnable: &'static MyRunnableType, bar: Bar) -> ! {
        my_runnable_type::tasks::run_task_2(runnable, bar).await
    }
    #[embassy_task]
    fn runner_3(runnable: &'static MyRunnableType, baz: Baz) -> ! {
        my_runnable_type::tasks::run_task_3(runnable, baz).await
    }

    spawner.must_spawn(runner_1(&instance, Foo::new( /* .. */ )));
    spawner.must_spawn(runner_2(&instance, Bar::new( /* .. */ )));
    spawner.must_spawn(runner_3(&instance, Baz::new( /* .. */ )));
}

```

Consider something like this:
```rust
///// Your type's definition /////
struct MyRunnableTypeInner { /* .. */ }

impl<'hw> MyRunnableTypeInner<'hw> {
    async fn task_1(&self, foo: Foo) -> ! { /* .. */ }
    async fn task_2(&self, bar: Bar) -> ! { /* .. */ }
    async fn task_3(&self, baz: Baz) -> ! { /* .. */ }
}

#[derive(Default)]
pub struct Resources<'hw> {
    inner: Option<MyRunnableTypeInner>
}

pub struct MyRunnableType<'hw> {
    inner: &'hw MyRunnableTypeInner
}

pub struct Runner<'hw> {
    inner: &'hw MyRunnableTypeInner,
    foo: Foo,
    bar: Bar,
    baz: Baz
}

impl<'hw> Runner<'hw> {
    pub async fn run(self) -> ! {
        loop {
            embassy_sync::join::join3(
                self.inner.task_1(self.foo),
                self.inner.task_2(self.bar),
                self.inner.task_3(self.baz)
            ).await;
        }
    }
}

impl<'hw> MyRunnableType<'hw> {
    pub fn new(resources: &mut Resources, foo: Foo, bar: Bar, baz: Baz /* .. */ ) -> (Self, Runner) {
        let inner = resources.insert(RunnableTypeInner::new( /* .. */ ));
        (Self { inner }, Runner { inner, foo, bar, baz })
    }
}

///// End-user code /////

fn main() {
    let (instance, runner) = MyRunnableType::new(/* .. */);
    #[embassy_task]
    fn runner_fn(runner: Runner) {
        runner.run().await
    }

    spawner.must_spawn(runner_fn(runner));
}
```
Notice that most of the complexity has been moved into internal implementation details and the client doesn't have to think about it.  Also notice that if you want to add a new 'green thread', or change what state is available to which 'green threads' you can do that entirely in private code in the `run()` method, without requiring changes to your client.

### Use traits for public methods expected to be used at run time whenever possible

In most cases, public APIs in this repo should be exposed in terms of traits rather than methods directly on the object, and objects that need to interact with other embedded-services objects should refer to them by trait rather than by name.  This does not apply to public methods used to construct or initialize a service, because those generally need to know something about the concrete implementation type to properly initialize it.

These traits should be defined in standalone 'interface' crates (i.e. `battery-service-interface`) alongside any support types needed for the interface (e.g. an Error enum)

__Reason__: Improved testability and customizability.
Testability - if all our types interact with each other via traits rather than direct dependencies on the type, it makes it much easier to mock out individual components.
Customizability - if an OEM needs to insert a special behavior, they can substitute in a different implementation of that trait and continue using the rest of the embedded-services code without modification.

__Example__:
Instead of
```rust
pub struct ExampleService { /* */ }
impl ExampleService {
    fn foo(&mut self) -> Result<()> { /* .. */ }
    fn bar(&mut self) -> Result<()> { /* .. */ }
    fn baz(&mut self) -> Result<()> { /* .. */ }
}
```

Consider:
```rust
// In a standalone interface crate
pub trait ExampleService {
    fn foo(&mut self) -> Result<()>;
    fn bar(&mut self) -> Result<()>;
    fn baz(&mut self) -> Result<()>;
}

// In the reference implementation crate
pub struct OdpExampleService { /* .. */ }

impl embedded_services::ExampleService for OdpExampleService {
    fn foo(&mut self) -> Result<()> { /* .. */ }
    fn bar(&mut self) -> Result<()> { /* .. */ }
    fn baz(&mut self) -> Result<()> { /* .. */ }
}
```