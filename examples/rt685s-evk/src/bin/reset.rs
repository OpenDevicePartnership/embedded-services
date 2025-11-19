#![no_std]
#![no_main]

extern crate rt685s_evk_example;

use platform_service::reset;
use {defmt_rtt as _, panic_probe as _};

async fn print_watcher_number() {
    static CONTRIVED: embassy_sync::mutex::Mutex<embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex, usize> =
        embassy_sync::mutex::Mutex::new(0);

    // yes, this could be accomplished with atomics. But using a mutex here demonstrates async functionality
    let watcher_num = {
        let mut current_number = CONTRIVED.lock().await;
        *current_number += 1;
        *current_number
    };

    defmt::info!("Reset Watcher #{}", watcher_num);
}

#[embassy_executor::task(pool_size = 10)]
async fn reset_watcher(blocker: &'static reset::Blocker) {
    defmt::info!("reset::Blocker watch thread ticking...");

    loop {
        blocker
            .wait_for_reset(async || {
                print_watcher_number().await;
            })
            .await;
    }
}

#[embassy_executor::main]
async fn main(spawner: embassy_executor::Spawner) {
    let _p = embassy_imxrt::init(Default::default());

    static BLOCKERS: embassy_sync::lazy_lock::LazyLock<[reset::Blocker; 10]> =
        embassy_sync::lazy_lock::LazyLock::new(|| [const { reset::Blocker::uninit() }; 10]);

    embedded_services::init().await;

    let blockers = BLOCKERS.get();

    // spawn blocker threads
    for blocker in blockers {
        // register before spawning blocker threads to avoid potential scheduling issues
        // when immediately calling reset below
        blocker.register().expect("Infallible");

        spawner.must_spawn(reset_watcher(blocker));
    }

    // perform reset
    defmt::info!("Performing platform reset!");
    reset::system_reset().await;
}
