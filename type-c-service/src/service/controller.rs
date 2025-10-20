use embedded_services::{
    debug, error,
    type_c::{
        ControllerId,
        external::{self, ControllerCommandData},
    },
};

use super::*;

impl<'a> Service<'a> {
    /// Process external controller status command
    pub(super) async fn process_external_controller_status(
        &self,
        controllers: &intrusive_list::IntrusiveList,
        controller: ControllerId,
    ) -> external::Response<'static> {
        let status = self.context.get_controller_status(controllers, controller).await;
        if let Err(e) = status {
            error!("Error getting controller status: {:#?}", e);
        }
        external::Response::Controller(status.map(external::ControllerResponseData::ControllerStatus))
    }

    /// Process external controller sync state command
    pub(super) async fn process_external_controller_sync_state(
        &self,
        controllers: &intrusive_list::IntrusiveList,
        controller: ControllerId,
    ) -> external::Response<'static> {
        let status = self.context.sync_controller_state(controllers, controller).await;
        if let Err(e) = status {
            error!("Error getting controller sync state: {:#?}", e);
        }
        external::Response::Controller(status.map(|_| external::ControllerResponseData::Complete))
    }

    /// Process external controller reset command
    pub(super) async fn process_external_controller_reset(
        &self,
        controllers: &intrusive_list::IntrusiveList,
        controller: ControllerId,
    ) -> external::Response<'static> {
        let status = self.context.reset_controller(controllers, controller).await;
        if let Err(e) = status {
            error!("Error resetting controller: {:#?}", e);
        }
        external::Response::Controller(status.map(|_| external::ControllerResponseData::Complete))
    }

    /// Process external controller commands
    pub(super) async fn process_external_controller_command(
        &self,
        controllers: &intrusive_list::IntrusiveList,
        command: &external::ControllerCommand,
    ) -> external::Response<'static> {
        debug!("Processing external controller command: {:#?}", command);
        match command.data {
            ControllerCommandData::ControllerStatus => {
                self.process_external_controller_status(controllers, command.id).await
            }
            ControllerCommandData::SyncState => {
                self.process_external_controller_sync_state(controllers, command.id)
                    .await
            }
            ControllerCommandData::Reset => self.process_external_controller_reset(controllers, command.id).await,
        }
    }
}
