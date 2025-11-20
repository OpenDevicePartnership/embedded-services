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
