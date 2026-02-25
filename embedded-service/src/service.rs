//! This module contains helper traits and functions for services that run on the EC.

/// A trait for a service that can be run on the EC.
/// Implementations of RunnableService should have an init() function to construct the service that
/// returns a Runner, which the user is expected to spawn a task for.
pub trait RunnableService<'hw>: Sized {
    /// A type that can be used to run the service. This is returned by the init() function and the user is
    /// expected to call its run() method in an embassy task (or similar parallel execution context on other
    /// async runtimes).
    type Runner: ServiceRunner<'hw>;

    /// The error type that your `init` function can return on failure.
    type ErrorType;

    /// Any initialization parameters that your service needs to run.
    type InitParams;

    /// Initializes an instance of the service in the provided OnceLock and returns a reference to the service and
    /// a runner that can be used to run the service.
    fn init(
        storage: &'hw embassy_sync::once_lock::OnceLock<Self>,
        params: Self::InitParams,
    ) -> impl core::future::Future<Output = Result<(&'hw Self, Self::Runner), Self::ErrorType>>;
}

/// A trait for a run handle used to execute a service's event loop.  This is returned by RunnableService::init()
/// and the user is expected to call its run() method in an embassy task (or similar parallel execution context
/// on other async runtimes).
pub trait ServiceRunner<'hw> {
    /// Run the service event loop. This future never completes.
    fn run(self) -> impl core::future::Future<Output = crate::Never> + 'hw;
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
