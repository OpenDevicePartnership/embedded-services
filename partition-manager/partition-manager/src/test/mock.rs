use core::fmt::Debug;
use std::collections::VecDeque;

#[derive(Debug, PartialEq)]
pub struct ActionRead {
    pub offset: u32,
    pub bytes: Vec<u8>,
}

#[derive(Debug, PartialEq)]
pub struct ActionWrite {
    pub offset: u32,
    pub bytes: Vec<u8>,
}

#[derive(Debug, PartialEq)]
pub struct ActionErase {
    pub offset: u32,
    pub len: u32,
}

#[derive(Debug, PartialEq)]
pub enum Action {
    Read(ActionRead),
    Write(ActionWrite),
    Erase(ActionErase),
}

impl From<ActionRead> for Action {
    fn from(value: ActionRead) -> Self {
        Action::Read(value)
    }
}

impl From<ActionWrite> for Action {
    fn from(value: ActionWrite) -> Self {
        Action::Write(value)
    }
}

impl From<ActionErase> for Action {
    fn from(value: ActionErase) -> Self {
        Action::Erase(value)
    }
}

#[allow(unused)]
#[derive(Debug, PartialEq)]
pub enum ActionVariant {
    Read,
    Write,
    Erase,
}

#[allow(unused)]
impl Action {
    pub fn variant(&self) -> ActionVariant {
        match self {
            Action::Read(_) => ActionVariant::Read,
            Action::Write(_) => ActionVariant::Write,
            Action::Erase(_) => ActionVariant::Erase,
        }
    }
}
#[allow(unused)]
pub struct MockDisk {
    pub size: usize,
    pub actions: VecDeque<Action>,
}

#[allow(unused)]
impl MockDisk {
    fn test(&mut self, action: Action) -> Option<Vec<u8>> {
        if let Some(expected) = self.actions.pop_front() {
            match (action, expected) {
                (Action::Read(action), Action::Read(expected)) => {
                    assert_eq!(action.offset, expected.offset);
                    assert_eq!(action.bytes.len(), expected.bytes.len());
                    return Some(expected.bytes);
                }
                (Action::Write(action), Action::Write(expected)) => {
                    assert_eq!(action.offset, expected.offset);
                    assert_eq!(action.bytes, expected.bytes);
                }
                (Action::Erase(action), Action::Erase(expected)) => {
                    assert_eq!(action.offset, expected.offset);
                    assert_eq!(action.len, expected.len);
                }
                (action, expected) => {
                    panic!("Expected {:?}, got {:?}", action, expected);
                }
            }

            None
        } else {
            panic!("Action {:?} performed on MockDisk, none remaining", action);
        }
    }

    fn test_read(&mut self, offset: u32, bytes: &mut [u8]) {
        let buf = self
            .test(
                ActionRead {
                    offset,
                    bytes: Vec::from(&*bytes),
                }
                .into(),
            )
            .unwrap();
        bytes.copy_from_slice(&buf);
    }

    fn test_write(&mut self, offset: u32, bytes: &[u8]) {
        self.test(
            ActionWrite {
                offset,
                bytes: Vec::from(bytes),
            }
            .into(),
        );
    }

    fn test_erase(&mut self, offset: u32, len: u32) {
        self.test(ActionErase { offset, len }.into());
    }

    pub fn check(self) {
        assert!(self.actions.is_empty());
    }
}

#[cfg(feature = "esa")]
pub mod esa {
    use super::*;
    use embedded_storage_async::nor_flash::{ErrorType, NorFlash, NorFlashError, NorFlashErrorKind, ReadNorFlash};

    #[derive(Debug, PartialEq, PartialOrd)]
    pub enum Error {
        NotAligned,
    }

    impl NorFlashError for Error {
        fn kind(&self) -> NorFlashErrorKind {
            match self {
                Error::NotAligned => NorFlashErrorKind::NotAligned,
            }
        }
    }

    impl ErrorType for &mut MockDisk {
        type Error = Error;
    }

    impl ReadNorFlash for &mut MockDisk {
        const READ_SIZE: usize = 4;

        async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
            if (offset as usize) % Self::READ_SIZE != 0 || bytes.len() % Self::READ_SIZE != 0 {
                return Err(Error::NotAligned);
            }

            self.test_read(offset, bytes);
            Ok(())
        }

        fn capacity(&self) -> usize {
            self.size
        }
    }

    impl NorFlash for &mut MockDisk {
        const WRITE_SIZE: usize = 128;
        const ERASE_SIZE: usize = 128;

        async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
            let len = to.checked_sub(from).unwrap();

            if (from as usize) % Self::ERASE_SIZE != 0 || len as usize % Self::ERASE_SIZE != 0 {
                return Err(Error::NotAligned);
            }

            self.test_erase(from, len);
            Ok(())
        }

        async fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
            if (offset as usize) % Self::WRITE_SIZE != 0 || bytes.len() % Self::WRITE_SIZE != 0 {
                return Err(Error::NotAligned);
            }

            self.test_write(offset, bytes);
            Ok(())
        }
    }
}

#[cfg(feature = "bdd")]
pub mod bdd {
    use super::*;
    use block_device_driver::{BlockDevice, blocks_to_slice, blocks_to_slice_mut};

    #[derive(Debug, PartialEq)]
    pub struct OutOfBounds;
    pub const BLOCK_SIZE: usize = 8;
    impl BlockDevice<BLOCK_SIZE> for &mut MockDisk {
        type Error = OutOfBounds;
        type Align = aligned::A1;

        async fn read(
            &mut self,
            block_address: u32,
            blocks: &mut [aligned::Aligned<Self::Align, [u8; BLOCK_SIZE]>],
        ) -> Result<(), Self::Error> {
            let end_block = block_address + blocks.len() as u32;
            if end_block * BLOCK_SIZE as u32 > self.size as u32 {
                return Err(OutOfBounds);
            }

            self.test_read(block_address * BLOCK_SIZE as u32, blocks_to_slice_mut(blocks));
            Ok(())
        }

        async fn write(
            &mut self,
            block_address: u32,
            blocks: &[aligned::Aligned<Self::Align, [u8; BLOCK_SIZE]>],
        ) -> Result<(), Self::Error> {
            let end_block = block_address + blocks.len() as u32;
            if end_block * BLOCK_SIZE as u32 > self.size as u32 {
                return Err(OutOfBounds);
            }

            let offset = block_address * BLOCK_SIZE as u32;
            let bytes = blocks_to_slice(blocks);

            self.test_erase(offset, bytes.len() as u32);
            self.test_write(offset, bytes);
            Ok(())
        }

        async fn size(&mut self) -> Result<u64, Self::Error> {
            Ok(self.size as u64)
        }
    }
}
