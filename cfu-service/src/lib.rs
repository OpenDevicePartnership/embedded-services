#![no_std]

use embassy_sync::once_lock::OnceLock;
use embedded_cfu_protocol::client::CfuReceiveContent;
use embedded_cfu_protocol::components::CfuComponentTraits;
use embedded_cfu_protocol::protocol_definitions::*;
use embedded_services::cfu::component::*;
use embedded_services::cfu::{CfuError, ContextToken};
use embedded_services::{comms, error, info, trace};

pub mod buffer;
pub mod host;
pub mod splitter;

pub struct CfuClient {
    /// Cfu Client context
    context: ContextToken,
    /// Comms endpoint
    tp: comms::Endpoint,
}

impl<T, C> CfuReceiveContent<T, C, ()> for CfuClient {
    async fn process_command(&self, _args: Option<T>, _cmd: C) -> Result<(), ()> {
        trace!("CfuClient CfuReceiveContent::process_command do nothing implementation.");
        Ok(())
    }

    async fn prepare_components(
        &self,
        _args: Option<T>,
        _primary_component: impl CfuComponentTraits,
    ) -> Result<(), ()> {
        trace!("CfuClient CfuReceiveContent::prepare_components do nothing implementation.");
        Ok(())
    }
}

impl CfuClient {
    /// Create a new Cfu Client
    pub fn create() -> Option<Self> {
        Some(Self {
            context: ContextToken::create()?,
            tp: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::Nonvol)),
        })
    }
    pub async fn process_request(&self) -> Result<(), CfuError> {
        let request = self.context.wait_request().await;
        //let device = self.context.get_device(request.id).await?;
        let comp = request.id;

        match request.data {
            RequestData::FwVersionRequest => {
                info!("Received FwVersionRequest, comp {}", comp);
                if let Ok(device) = self.context.get_device(comp).await {
                    let resp = device
                        .execute_device_request(request.data)
                        .await
                        .map_err(CfuError::ProtocolError)?;

                    // TODO replace with signal to component to get its own fw version
                    //cfu::send_request(comp, RequestData::FwVersionRequest).await?;
                    match resp {
                        InternalResponseData::FwVersionResponse(r) => {
                            let ver = r.component_info[0].fw_version;
                            info!("got fw version {:?} for comp {}", ver, comp);
                        }
                        _ => {
                            error!("Invalid response to get fw version {:?} from comp {}", resp, comp);
                            return Err(CfuError::ProtocolError(CfuProtocolError::BadResponse));
                        }
                    }
                    self.context.send_response(resp).await;
                    return Ok(());
                }
                Err(CfuError::InvalidComponent)
            }
            RequestData::GiveContent(_content_cmd) => Ok(()),
            RequestData::GiveOffer(_offer_cmd) => Ok(()),
            RequestData::PrepareComponentForUpdate => Ok(()),
            RequestData::FinalizeUpdate => Ok(()),
            RequestData::GiveOfferExtended(_) => {
                // Don't currently support extended offers
                self.context
                    .send_response(InternalResponseData::OfferResponse(
                        FwUpdateOfferResponse::new_with_failure(
                            HostToken::Driver,
                            OfferRejectReason::InvalidComponent,
                            OfferStatus::Reject,
                        ),
                    ))
                    .await;
                Ok(())
            }
            RequestData::GiveOfferInformation(_) => {
                // Don't currently support information offers
                self.context
                    .send_response(InternalResponseData::OfferResponse(
                        FwUpdateOfferResponse::new_with_failure(
                            HostToken::Driver,
                            OfferRejectReason::InvalidComponent,
                            OfferStatus::Reject,
                        ),
                    ))
                    .await;
                Ok(())
            }
        }
    }
}

impl comms::MailboxDelegate for CfuClient {}

#[embassy_executor::task]
pub async fn task() {
    info!("Starting cfu client task");
    static CLIENT: OnceLock<CfuClient> = OnceLock::new();
    let cfuclient = CLIENT.get_or_init(|| CfuClient::create().expect("cfu client singleton already initialized"));

    if comms::register_endpoint(cfuclient, &cfuclient.tp).await.is_err() {
        error!("Failed to register cfu endpoint");
        return;
    }

    loop {
        if let Err(e) = cfuclient.process_request().await {
            error!("Error processing request: {:?}", e);
        }
    }
}
