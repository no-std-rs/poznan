//! File-backed NOR emulation: seek/read, aligned writes, erase and durable flush.
use embedded_storage::nor_flash::{ErrorType, NorFlash, NorFlashErrorKind, ReadNorFlash};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};
pub const SIZE: usize = 65536;
pub struct Flash(File);
impl Flash {
    pub fn open(path: &Path) -> io::Result<Self> {
        let created = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path);
        let mut file = match created {
            Ok(mut f) => {
                for _ in 0..SIZE / 256 {
                    f.write_all(&[0xff; 256])?;
                }
                f.sync_all()?;
                f
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                OpenOptions::new().read(true).write(true).open(path)?
            }
            Err(e) => return Err(e),
        };
        if file.seek(SeekFrom::End(0))? != SIZE as u64 {
            return Err(io::Error::other("cache file must be exactly 65536 bytes"));
        }
        Ok(Self(file))
    }
    fn seek(&mut self, offset: u32, n: usize, alignment: usize) -> Result<(), NorFlashErrorKind> {
        if !(offset as usize).is_multiple_of(alignment) || !n.is_multiple_of(alignment) {
            return Err(NorFlashErrorKind::NotAligned);
        }
        if u64::from(offset) + n as u64 > SIZE as u64 {
            return Err(NorFlashErrorKind::OutOfBounds);
        }
        self.0
            .seek(SeekFrom::Start(offset.into()))
            .map_err(|_| NorFlashErrorKind::Other)?;
        Ok(())
    }
}
impl ErrorType for Flash {
    type Error = NorFlashErrorKind;
}
impl ReadNorFlash for Flash {
    const READ_SIZE: usize = 4;
    fn read(&mut self, offset: u32, out: &mut [u8]) -> Result<(), Self::Error> {
        self.seek(offset, out.len(), 4)?;
        self.0.read_exact(out).map_err(|_| NorFlashErrorKind::Other)
    }
    fn capacity(&self) -> usize {
        SIZE
    }
}
impl NorFlash for Flash {
    const WRITE_SIZE: usize = 4;
    const ERASE_SIZE: usize = 4096;
    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        self.seek(offset, bytes.len(), 4)?;
        self.0
            .write_all(bytes)
            .and_then(|_| self.0.sync_data())
            .map_err(|_| NorFlashErrorKind::Other)
    }
    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        let n = to.checked_sub(from).ok_or(NorFlashErrorKind::OutOfBounds)? as usize;
        self.seek(from, n, 4096)?;
        for _ in 0..n / 256 {
            self.0
                .write_all(&[0xff; 256])
                .map_err(|_| NorFlashErrorKind::Other)?;
        }
        self.0.sync_data().map_err(|_| NorFlashErrorKind::Other)
    }
}
