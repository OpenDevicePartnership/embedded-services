pub async fn fan_task<'hw, T: crate::fan::Controller, const SAMPLE_BUF_LEN: usize>(
    fan: &crate::fan::Fan<T, SAMPLE_BUF_LEN>,
    thermal_service: &crate::Service<'hw>,
) {
    let _ = embassy_futures::join::join3(
        fan.handle_rx(),
        fan.handle_sampling(),
        fan.handle_auto_control(thermal_service),
    )
    .await;
}

pub async fn sensor_task<'hw, T: crate::sensor::Controller, const SAMPLE_BUF_LEN: usize>(
    sensor: &crate::sensor::Sensor<T, SAMPLE_BUF_LEN>,
    thermal_service: &crate::Service<'hw>,
) {
    let _ = embassy_futures::join::join(sensor.handle_rx(), sensor.handle_sampling(thermal_service)).await;
}
