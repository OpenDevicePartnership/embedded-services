//! Module for a mock that implements [`fw_update_interface::basic::FwUpdate`]
use std::collections::VecDeque;
use std::vec::Vec;

use embedded_services::named::Named;
use fw_update_interface::basic::{Error, FwUpdate};

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FnCall {
    GetActiveFwVersion,
    StartFwUpdate,
    AbortFwUpdate,
    FinalizeFwUpdate,
    WriteFwContents(usize, Vec<u8>),
}

pub struct Mock {
    /// Signal to record function calls
    pub fn_calls: VecDeque<FnCall>,
    /// The next error to return from the mock
    next_error: Option<Error>,
    /// Mock current FW version
    current_fw_version: u32,
    /// Human-readable name of the mock
    name: &'static str,
}

impl Mock {
    pub fn new(name: &'static str, current_fw_version: u32) -> Self {
        Self {
            name,
            fn_calls: VecDeque::new(),
            next_error: None,
            current_fw_version,
        }
    }

    fn record_fn_call(&mut self, fn_call: FnCall) {
        self.fn_calls.push_back(fn_call);
    }

    /// Set an error for the next function call
    pub fn set_next_error(&mut self, error: Option<Error>) {
        self.next_error = error;
    }
}

impl FwUpdate for Mock {
    async fn get_active_fw_version(&mut self) -> Result<u32, Error> {
        self.record_fn_call(FnCall::GetActiveFwVersion);
        if let Some(error) = self.next_error.take() {
            return Err(error);
        }
        Ok(self.current_fw_version)
    }

    async fn start_fw_update(&mut self) -> Result<(), Error> {
        self.record_fn_call(FnCall::StartFwUpdate);
        if let Some(error) = self.next_error.take() {
            return Err(error);
        }
        Ok(())
    }

    async fn abort_fw_update(&mut self) -> Result<(), Error> {
        self.record_fn_call(FnCall::AbortFwUpdate);
        if let Some(error) = self.next_error.take() {
            return Err(error);
        }
        Ok(())
    }

    async fn finalize_fw_update(&mut self) -> Result<(), Error> {
        self.record_fn_call(FnCall::FinalizeFwUpdate);
        if let Some(error) = self.next_error.take() {
            return Err(error);
        }
        Ok(())
    }

    async fn write_fw_contents(&mut self, offset: usize, data: &[u8]) -> Result<(), Error> {
        self.record_fn_call(FnCall::WriteFwContents(offset, Vec::from(data)));

        if let Some(error) = self.next_error.take() {
            return Err(error);
        }
        Ok(())
    }
}

impl Named for Mock {
    fn name(&self) -> &'static str {
        self.name
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::vec;

    #[tokio::test]
    async fn test_get_active_fw_version() {
        let mut mock = super::Mock::new("test", 1);
        let version = mock.get_active_fw_version().await;
        assert_eq!(version, Ok(1));
        assert_eq!(mock.fn_calls.pop_front(), Some(FnCall::GetActiveFwVersion));
    }

    #[tokio::test]
    async fn test_start_fw_update() {
        let mut mock = super::Mock::new("test", 1);
        let result = mock.start_fw_update().await;
        assert_eq!(result, Ok(()));
        assert_eq!(mock.fn_calls.pop_front(), Some(FnCall::StartFwUpdate));
    }

    #[tokio::test]
    async fn test_abort_fw_update() {
        let mut mock = super::Mock::new("test", 1);
        let result = mock.abort_fw_update().await;
        assert_eq!(result, Ok(()));
        assert_eq!(mock.fn_calls.pop_front(), Some(FnCall::AbortFwUpdate));
    }

    #[tokio::test]
    async fn test_finalize_fw_update() {
        let mut mock = super::Mock::new("test", 1);
        let result = mock.finalize_fw_update().await;
        assert_eq!(result, Ok(()));
        assert_eq!(mock.fn_calls.pop_front(), Some(FnCall::FinalizeFwUpdate));
    }

    #[tokio::test]
    async fn test_write_fw_contents() {
        let mut mock = super::Mock::new("test", 1);
        let data = vec![1, 2, 3, 4];
        let result = mock.write_fw_contents(0, &data).await;
        assert_eq!(result, Ok(()));
        assert_eq!(mock.fn_calls.pop_front(), Some(FnCall::WriteFwContents(0, data)));
    }

    #[tokio::test]
    async fn test_set_next_error() {
        let mut mock = super::Mock::new("test", 1);
        mock.set_next_error(Some(Error::Failed));
        let result = mock.get_active_fw_version().await;
        assert_eq!(result, Err(Error::Failed));
    }
}
