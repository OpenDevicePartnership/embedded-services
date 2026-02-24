//! This module contains helper traits and functions for services that run on the EC.

/// A trait for a service that can be run on the EC.
/// Implementations of RunnableService should have an init() function to construct the service that
/// returns a Runner, which the user is expected to spawn a task for.
pub trait RunnableService<'hw>: Sized {
    /// A token type used to restrict users from spawning more than one service runner.  Services will generally
    /// define this as a zero-sized type and only provide a constructor for it that is private to the service module,
    /// which prevents users from constructing their own tokens and spawning multiple runners.
    /// Most services should consider using the `impl_runner_creation_token!` macro to do this automatically.
    type Runner: ServiceRunner<'hw>;

    /// The error type that your `init` function can return on failure.
    type ErrorType;

    /// Any initialization parameters that your service needs to run.
    type InitParams;

    /// Initializes an instance of the service in the provided OnceLock and returns a reference to the service and
    /// a runner that can be used to run the service.
    fn init(
        storage: &'hw embassy_sync::once_lock::OnceLock<Self>, // TODO could be resources?
        params: Self::InitParams,
    ) -> impl core::future::Future<Output = Result<(&'hw Self, Self::Runner), Self::ErrorType>>;
}

/// A trait for a run handle used to execute a service's event loop.  This is returned by RunnableService::init()
/// and the user is expected to call its run() method in an embassy task (or similar parallel execution context
/// on other async runtimes).
pub trait ServiceRunner<'hw> {
    /// Run the service event loop. This future never completes.
    fn run(self) -> impl core::future::Future<Output = crate::Never> + 'hw;
    // TODO: Do we want to take &mut self instead of consuming self? I think the difference is that it allows for the possibility of
    //       the user select!()ing over the ServiceRunner and something else, then having that other thing complete and bailing
    //       out of execution. In the consume-self version, the user can't restart afterward, but in the &mut self version they could
    //       potentially restart the runner.  It's not clear to me if we have any use cases for the 'restartable runner' version, and if
    //       we don't then the consume-self version more clearly telegraphs the fact that the runner is not meant to be restarted or
    //       reused after it's started and lets the implementor care less about drop safety on the future
}

/// Initializes a service, creates an embassy task to run it, and spawns that task.
///
/// This macro handles the boilerplate of:
/// 1. Creating a `static` [`OnceLock`](embassy_sync::once_lock::OnceLock) to hold the service
/// 2. Calling the service's `init()` method
/// 3. Defining an embassy_executor::task to run the service
/// 4. Spawning the task on the provided executor
///
/// Returns a Result<reference-to-service, Error> where Error is the error type of $service_ty::init().
///
/// Arguments
///
/// - spawner:    An embassy_executor::Spawner.
/// - service_ty: The service type that implements RunnableService that you want to create and run.
/// - init_arg:   The init argument type to pass to `Service::init()`
///
/// Example:
///
/// ```ignore
/// let time_service = embedded_services::spawn_service!(
///     spawner,
///     time_alarm_service::Service<'static>,
///     time_alarm_service::ServiceInitParams { dt_clock, tz, ac_expiration, ac_policy, dc_expiration, dc_policy }
/// ).expect("failed to initialize time_alarm service");
/// ```
#[macro_export]
macro_rules! spawn_service {
    ($spawner:expr, $service_ty:ty, $init_arg:expr) => {{
        use $crate::service::RunnableService;
        use $crate::service::ServiceRunner;
        static SERVICE: embassy_sync::once_lock::OnceLock<$service_ty> = embassy_sync::once_lock::OnceLock::new();
        match <$service_ty>::init(&SERVICE, $init_arg).await {
            Ok((service_ref, runner)) => {
                #[embassy_executor::task]
                async fn service_task_fn(runner: <$service_ty as $crate::service::RunnableService<'static>>::Runner) {
                    runner.run().await;
                }

                $spawner.must_spawn(service_task_fn(runner));
                Ok(service_ref)
            }
            Err(e) => Err(e),
        }
    }};
}

pub use spawn_service;
