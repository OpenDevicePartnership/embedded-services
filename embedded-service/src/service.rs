//! This module contains helper traits and functions for services that run on the EC.

/// A trait for a service that can be run on the EC.
/// Implementations of RunnableService should have an init() function to construct the service that
/// returns a Runner, which the user is expected to spawn a task for.
pub trait RunnableService<'hw>: Sized {
    /// A token type used to restrict users from spawning more than one service runner.  Services will generally
    /// define this as a zero-sized type and only provide a constructor for it that is private to the service module,
    /// which prevents users from constructing their own tokens and spawning multiple runners.
    /// Most services should consider using the `impl_runner_creation_token!` macro to do this automatically.
    type RunnerCreationToken;

    /// Run the service event loop. This future never completes.
    fn run(
        &'hw self,
        _creation_token: Self::RunnerCreationToken,
    ) -> impl core::future::Future<Output = crate::Never> + 'hw;

    // ##### NOTE - below this line is only needed to get typesafety for spawn_service!(), which we could do without  #####

    /// The error type that your `init` function can return on failure.
    type ErrorType;

    /// Any initialization parameters that your service needs to run.
    type InitParams;

    /// Initializes an instance of the service in the provided OnceLock and returns a reference to the service and
    /// a runner that can be used to run the service.
    fn init(
        storage: &'hw embassy_sync::once_lock::OnceLock<Self>,
        params: Self::InitParams,
    ) -> impl core::future::Future<Output = Result<(&'hw Self, ServiceRunner<'hw, Self>), Self::ErrorType>>;
}

/// A handle that must be passed to a spawned task and `.run().await`'d to drive the service.
/// Dropping this without calling `runner.run().await` means the service will not process events
pub struct ServiceRunner<'hw, T: RunnableService<'hw>> {
    service: &'hw T,
    creation_token: T::RunnerCreationToken, // This token is used to ensure that only the service can create a runner for itself. It's probably a zero-sized type.
}

impl<'hw, T: RunnableService<'hw>> ServiceRunner<'hw, T> {
    /// Runs the service event loop. This future never completes.
    pub async fn run(self) -> crate::Never {
        self.service.run(self.creation_token).await
    }

    /// Constructs a new service runner.  This is something the service will do in its init function; users of
    /// the service should not need to call this directly.
    pub fn new(service: &'hw T, token: T::RunnerCreationToken) -> Self {
        Self {
            service,
            creation_token: token,
        }
    }
}

/// Generates a default implementation of a runner creation token for a service.  This token is used to ensure that
/// only the service can create a runner for itself, and therefore it can control the number of tasks that a user is
/// allowed to spawn to run the service (e.g. if the service is not designed to be run by multiple tasks, it can use
/// this token to prevent that).
///
/// Most services will want to use this macro to generate a simple zero-sized token type - it needs to be a macro invoked
/// in the service module rather than a generic type in this module because the constructor needs to be private to the
/// service module to prevent users from constructing their own tokens and spawning multiple runners.
///
/// Arguments:
///   - token_name: The name of the token type to generate.
#[macro_export]
macro_rules! impl_runner_creation_token {
    ($token_name:ident) => {
        /// A token type used to restrict users from spawning more than one service runner.
        pub struct $token_name {
            _private: (),
        }

        impl $token_name {
            fn new() -> Self {
                Self { _private: () }
            }
        }
    };
}

pub use impl_runner_creation_token;

/// Initializes a service, creates an embassy task to run it, and spawns that task.
///
/// This macro handles the boilerplate of:
/// 1. Creating a `static` [`OnceLock`](embassy_sync::once_lock::OnceLock) to hold the service
/// 2. Calling the service's `init()` method
/// 3. Defining an [`embassy_executor::task`] to run the service
/// 4. Spawning the task on the provided executor
///
/// Returns a Result<reference-to-service, Error> where Error is the error type of $service_ty::init().
///
/// Note that for a service to be supported, it must have the following properties: // TODO figure out if this should be a trait. Would require a single associated-type arg rather than letting each service define its own init list though...
/// 1. Implements the RunnableService trait
/// 2. Has an init() function with the following properties:
///   i.  Takes as its first argument a &OnceLock<service_ty>
///   ii. Returns a Result<(reference-to-service, service-runner), Error> where the service-runner
///       is an instance of RunnableService.
///
/// Arguments
///
/// - spawner:    An [`embassy_executor::Spawner`].
/// - service_ty: The service type, wrapped in brackets to allow generic arguments
///   (e.g. `[my_crate::Service<'static>]`).
/// - init_args:  The arguments to pass to `Service::init()`, excluding the `OnceLock` argument, which is codegenned.
///
/// Example:
///
/// ```ignore
/// let time_service = embedded_services::spawn_service!(
///     time_alarm_task,
///     spawner,
///     [time_alarm_service::Service<'static>],
///     dt_clock, tz, ac_expiration, ac_policy, dc_expiration, dc_policy
/// ).expect("failed to initialize time_alarm service");
/// ```
#[macro_export]
macro_rules! spawn_service {
    ($spawner:expr, [ $($service_ty:tt)* ], $($init_args:expr),* $(,)?) => {
        {
            use embedded_services::service::RunnableService;
            static SERVICE: embassy_sync::once_lock::OnceLock<$($service_ty)*> = embassy_sync::once_lock::OnceLock::new();
            match <$($service_ty)*>::init(
                &SERVICE,
                $($init_args),*
            )
            .await {
                Ok((service_ref, runner)) => {
                    #[embassy_executor::task]
                    async fn service_task_fn(
                        runner: $crate::service::ServiceRunner<'static, $($service_ty)*>,
                    ) {
                        runner.run().await;
                    }

                    $spawner.must_spawn(service_task_fn(runner));
                    Ok(service_ref)
                },
                Err(e) => Err(e)
            }
        }
    };
}

pub use spawn_service;
