use embedded_storage::nor_flash::{ErrorType, NorFlash, NorFlashErrorKind, ReadNorFlash};

pub struct Flash<'a> {
    pub bytes: &'a mut [u8],
    pub operations: usize,
    /// Fail this mutation after changing the first `prefix` bytes.
    pub fail: Option<(usize, usize)>,
}
impl<'a> Flash<'a> {
    pub fn new(bytes: &'a mut [u8]) -> Self {
        Self {
            bytes,
            operations: 0,
            fail: None,
        }
    }
    fn limit(&mut self, n: usize) -> Result<usize, NorFlashErrorKind> {
        let op = self.operations;
        self.operations += 1;
        Ok(match self.fail {
            Some((i, prefix)) if op == i => prefix.min(n),
            _ => n,
        })
    }
}
impl ErrorType for Flash<'_> {
    type Error = NorFlashErrorKind;
}
impl ReadNorFlash for Flash<'_> {
    const READ_SIZE: usize = 4;
    fn read(&mut self, offset: u32, out: &mut [u8]) -> Result<(), Self::Error> {
        let a = offset as usize;
        if !a.is_multiple_of(4) || !out.len().is_multiple_of(4) {
            return Err(NorFlashErrorKind::NotAligned);
        }
        out.copy_from_slice(
            self.bytes
                .get(a..a + out.len())
                .ok_or(NorFlashErrorKind::OutOfBounds)?,
        );
        Ok(())
    }
    fn capacity(&self) -> usize {
        self.bytes.len()
    }
}
impl NorFlash for Flash<'_> {
    const WRITE_SIZE: usize = 4;
    const ERASE_SIZE: usize = 4096;
    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        let a = offset as usize;
        if !a.is_multiple_of(4) || !bytes.len().is_multiple_of(4) {
            return Err(NorFlashErrorKind::NotAligned);
        }
        let n = self.limit(bytes.len())?;
        let to = self
            .bytes
            .get_mut(a..a + bytes.len())
            .ok_or(NorFlashErrorKind::OutOfBounds)?;
        assert!(
            to.iter().all(|b| *b == 0xff),
            "rewrote a NOR word without erase"
        );
        to[..n].copy_from_slice(&bytes[..n]);
        if self.fail.is_some_and(|(i, _)| i + 1 == self.operations) {
            return Err(NorFlashErrorKind::Other);
        }
        Ok(())
    }
    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        if !from.is_multiple_of(4096) || !to.is_multiple_of(4096) {
            return Err(NorFlashErrorKind::NotAligned);
        }
        let n = self.limit((to - from) as usize)?;
        self.bytes[from as usize..from as usize + n].fill(0xff);
        if self.fail.is_some_and(|(i, _)| i + 1 == self.operations) {
            return Err(NorFlashErrorKind::Other);
        }
        Ok(())
    }
}
