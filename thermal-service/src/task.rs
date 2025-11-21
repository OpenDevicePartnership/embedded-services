use embedded_services::{comms, error};

use crate::{self as ts, mptf::process_request};

pub async fn handle_requests() {
    loop {
        let mut request = ts::wait_mctp_payload().await;
        process_request(&mut request).await;
        let send_result = ts::send_service_msg(
            comms::EndpointID::External(comms::External::Host),
            &embedded_services::ec_type::message::HostMsg::Response(request),
        )
        .await;

        if send_result.is_err() {
            error!("Failed to send response to MPTF request!");
        }
    }
}

pub async fn fan_task<T: crate::fan::Controller, const SAMPLE_BUF_LEN: usize>(
    fan: &'static crate::fan::Fan<T, SAMPLE_BUF_LEN>,
) {
    let _ = embassy_futures::join::join3(fan.handle_rx(), fan.handle_sampling(), fan.handle_auto_control()).await;
}

pub async fn sensor_task<T: crate::sensor::Controller, const SAMPLE_BUF_LEN: usize>(
    sensor: &'static crate::sensor::Sensor<T, SAMPLE_BUF_LEN>,
) {
    let _ = embassy_futures::join::join(sensor.handle_rx(), sensor.handle_sampling()).await;
}
