//! This module contains helper traits and functions for services that run on the EC.

/// A trait for a service that requires the caller to launch a long-running task on its behalf to operate.
pub trait Service<'hw>: Sized {
    /// A type that can be used to run the service. This is returned by the service's constructor and the user
    /// is expected to call its run() method in an embassy task (or similar parallel execution context on
    /// other async runtimes).
    type Runner: ServiceRunner<'hw>;

    /// Any memory resources that your service needs.  This is typically an opaque type that is only used by the service
    /// and is not interacted with by users of the service. Must be default-constructible for spawn_service!() to work.
    type Resources: Default;
}

/// A trait for a run handle used to execute a service's event loop.  This is returned by a service's
/// constructor and the user is expected to call its run() method in an embassy task (or similar parallel
/// execution context on other async runtimes).
pub trait ServiceRunner<'hw> {
    /// Run the service event loop. This future never completes.
    fn run(self) -> impl core::future::Future<Output = embedded_services::Never> + 'hw;
}

/// Initializes a service, creates an embassy task to run it, and spawns that task.
///
/// This macro handles the boilerplate of:
/// 1. Creating a `static` [`StaticCell`](static_cell::StaticCell) to hold the service's resources
/// 2. Invoking the caller-provided initialization closure to construct the service
/// 3. Defining an embassy_executor::task to run the service
/// 4. Spawning the task on the provided executor
///
/// Returns a `Result<Service, Error>` where `Error` is the error type produced by the initialization closure.
///
/// Arguments
///
/// - spawner:    An embassy_executor::Spawner.
/// - service_ty: The service type that implements Service that you want to create and run.
/// - init_fn:    A function that takes a `&'static mut Resources` and returns an async future that
///               returns a `Result<(Service, Runner), Error>`
///               The function is typically a closure that's just a thin wrapper that calls the service's
///               actual constructor with the provided resources.
///
/// Example:
///
/// ```ignore
/// let time_service = odp_service_common::runnable_service::spawn_service!(
///     spawner,
///     time_alarm_service::Service<'static>,
///     |resources| time_alarm_service::Service::new(
///         resources,
///         dt_clock, tz, ac_expiration, ac_policy, dc_expiration, dc_policy
///     )
/// ).expect("failed to initialize time_alarm service");
/// ```
#[macro_export]
macro_rules! spawn_service {
    ($spawner:expr, $service_ty:ty, $init_fn:expr) => {{
        use $crate::runnable_service::{Service, ServiceRunner};
        static SERVICE_RESOURCES: static_cell::StaticCell<<$service_ty as Service<'static>>::Resources> =
            static_cell::StaticCell::new();
        let service_resources =
            SERVICE_RESOURCES.init(<<$service_ty as Service<'static>>::Resources as Default>::default());

        #[embassy_executor::task]
        async fn service_task_fn(runner: <$service_ty as $crate::runnable_service::Service<'static>>::Runner) {
            runner.run().await;
        }

        // Coerce init_fn to an `FnOnce` so it can capture values from the surrounding scope
        fn call_once<F, Fut, T, E>(resources: &'static mut <$service_ty as Service<'static>>::Resources, f: F) -> Fut
        where
            F: FnOnce(&'static mut <$service_ty as Service<'static>>::Resources) -> Fut,
            Fut: core::future::Future<Output = Result<T, E>>,
        {
            f(resources)
        }

        call_once(service_resources, $init_fn)
            .await
            .map(|(control_handle, runner)| {
                $spawner.spawn(service_task_fn(runner).expect("Failed to spawn service task"));
                control_handle
            })
    }};
}

pub use spawn_service;
