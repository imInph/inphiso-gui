//! Byte-addressable access to a window of a raw device.
//!
//! The FAT library reads and writes a few bytes at a time (a directory entry,
//! a FAT cell), but raw devices only take whole, aligned sectors. This keeps a
//! small write-back cache of large aligned blocks: sequential file data fills
//! whole blocks and goes out in one write without being read first; partial
//! sector updates read the block once and patch it.

use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::blockio::AlignedBuf;
use crate::write_raw::Target;

pub const BLOCK: usize = 1 << 20;
const SLOTS: usize = 32;

struct Slot {
    /// Block index within the window.
    idx: u64,
    buf: AlignedBuf,
    /// The whole block was read from the device.
    loaded: bool,
    dirty: bool,
    /// Sectors fully overwritten since the block was cached (only tracked while not loaded).
    written: Vec<u64>,
    used: u64,
}

pub struct BlockCache<'a, D: Target> {
    dev: &'a mut D,
    base: u64,
    len: u64,
    sector: u64,
    pos: u64,
    slots: Vec<Slot>,
    tick: u64,
}

impl<'a, D: Target> BlockCache<'a, D> {
    /// A view of `len` bytes starting at `base`. Both must be multiples of `sector`.
    pub fn new(dev: &'a mut D, base: u64, len: u64, sector: u64) -> Self {
        assert!(base.is_multiple_of(sector) && len.is_multiple_of(sector));
        assert!((BLOCK as u64).is_multiple_of(sector));
        Self {
            dev,
            base,
            len,
            sector,
            pos: 0,
            slots: Vec::new(),
            tick: 0,
        }
    }

    fn sectors_per_block(&self) -> usize {
        BLOCK / self.sector as usize
    }

    /// Bytes of the window covered by block `idx` (the last block may be short).
    fn block_len(&self, idx: u64) -> usize {
        (self.len - idx * BLOCK as u64).min(BLOCK as u64) as usize
    }

    fn read_from_device(&mut self, idx: u64, buf: &mut [u8]) -> io::Result<()> {
        let n = self.block_len(idx);
        self.dev
            .seek(SeekFrom::Start(self.base + idx * BLOCK as u64))?;
        self.dev.read_exact(&mut buf[..n])
    }

    fn write_slot(&mut self, i: usize) -> io::Result<()> {
        if !self.slots[i].dirty {
            return Ok(());
        }
        let idx = self.slots[i].idx;
        let n = self.block_len(idx);
        // Sectors nobody wrote still hold whatever the device had; fetch them first.
        if !self.slots[i].loaded && !self.fully_written(i) {
            self.ensure_loaded(i)?;
        }
        self.dev
            .seek(SeekFrom::Start(self.base + idx * BLOCK as u64))?;
        self.dev.write_all(&self.slots[i].buf[..n])?;
        self.slots[i].dirty = false;
        Ok(())
    }

    fn fully_written(&self, i: usize) -> bool {
        let sectors = self.block_len(self.slots[i].idx) / self.sector as usize;
        (0..sectors).all(|sec| self.slots[i].written[sec / 64] & (1 << (sec % 64)) != 0)
    }

    /// Returns the slot holding block `idx`, evicting the least recently used if needed.
    fn slot_for(&mut self, idx: u64) -> io::Result<usize> {
        self.tick += 1;
        if let Some(i) = self.slots.iter().position(|s| s.idx == idx) {
            self.slots[i].used = self.tick;
            return Ok(i);
        }
        let words = self.sectors_per_block().div_ceil(64);
        let i = if self.slots.len() < SLOTS {
            self.slots.push(Slot {
                idx,
                buf: AlignedBuf::new(BLOCK),
                loaded: false,
                dirty: false,
                written: vec![0; words],
                used: 0,
            });
            self.slots.len() - 1
        } else {
            let lru = (0..self.slots.len())
                .min_by_key(|&i| self.slots[i].used)
                .unwrap();
            self.write_slot(lru)?;
            lru
        };
        let slot = &mut self.slots[i];
        slot.idx = idx;
        slot.loaded = false;
        slot.dirty = false;
        slot.written.iter_mut().for_each(|w| *w = 0);
        slot.used = self.tick;
        Ok(i)
    }

    fn ensure_loaded(&mut self, i: usize) -> io::Result<()> {
        if self.slots[i].loaded {
            return Ok(());
        }
        let idx = self.slots[i].idx;
        let mut disk = AlignedBuf::new(BLOCK);
        self.read_from_device(idx, &mut disk)?;
        let s = self.sector as usize;
        let n = self.block_len(idx);
        let slot = &mut self.slots[i];
        // Keep what was already written into the cache; take the rest from disk.
        for sec in 0..n / s {
            if slot.written[sec / 64] & (1 << (sec % 64)) == 0 {
                slot.buf[sec * s..(sec + 1) * s].copy_from_slice(&disk[sec * s..(sec + 1) * s]);
            }
        }
        slot.loaded = true;
        Ok(())
    }

    /// Writes every dirty block, in order, then syncs the device.
    pub fn flush_all(&mut self) -> io::Result<()> {
        let mut order: Vec<usize> = (0..self.slots.len()).collect();
        order.sort_by_key(|&i| self.slots[i].idx);
        for i in order {
            self.write_slot(i)?;
        }
        self.dev.sync()
    }
}

impl<D: Target> Read for BlockCache<'_, D> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.len || buf.is_empty() {
            return Ok(0);
        }
        let idx = self.pos / BLOCK as u64;
        let off = (self.pos % BLOCK as u64) as usize;
        let n = buf
            .len()
            .min(self.block_len(idx) - off)
            .min((self.len - self.pos) as usize);
        let i = self.slot_for(idx)?;
        self.ensure_loaded(i)?;
        buf[..n].copy_from_slice(&self.slots[i].buf[off..off + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl<D: Target> Write for BlockCache<'_, D> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.pos >= self.len {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "past the end of the partition",
            ));
        }
        let idx = self.pos / BLOCK as u64;
        let off = (self.pos % BLOCK as u64) as usize;
        let n = buf
            .len()
            .min(self.block_len(idx) - off)
            .min((self.len - self.pos) as usize);
        let i = self.slot_for(idx)?;
        let s = self.sector as usize;
        let whole_sectors = off.is_multiple_of(s) && n.is_multiple_of(s);
        if !whole_sectors {
            self.ensure_loaded(i)?;
        }
        let slot = &mut self.slots[i];
        slot.buf[off..off + n].copy_from_slice(&buf[..n]);
        if !slot.loaded {
            for sec in off / s..(off + n) / s {
                slot.written[sec / 64] |= 1 << (sec % 64);
            }
        }
        slot.dirty = true;
        self.pos += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_all()
    }
}

impl<D: Target> Seek for BlockCache<'_, D> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new = match pos {
            SeekFrom::Start(p) => p as i128,
            SeekFrom::End(d) => self.len as i128 + d as i128,
            SeekFrom::Current(d) => self.pos as i128 + d as i128,
        };
        if new < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before start",
            ));
        }
        self.pos = new as u64;
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A device that rejects anything not sector-aligned, like a raw disk.
    struct StrictDisk {
        data: Vec<u8>,
        pos: u64,
        sector: u64,
        writes: usize,
    }

    impl StrictDisk {
        fn check(&self, len: usize) -> io::Result<()> {
            if !self.pos.is_multiple_of(self.sector) || !(len as u64).is_multiple_of(self.sector) {
                return Err(io::Error::other(format!(
                    "unaligned I/O at {} len {len}",
                    self.pos
                )));
            }
            Ok(())
        }
    }

    impl Read for StrictDisk {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.check(buf.len())?;
            let p = self.pos as usize;
            let n = buf.len().min(self.data.len() - p);
            buf[..n].copy_from_slice(&self.data[p..p + n]);
            self.pos += n as u64;
            Ok(n)
        }
    }

    impl Write for StrictDisk {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.check(buf.len())?;
            let p = self.pos as usize;
            self.data[p..p + buf.len()].copy_from_slice(buf);
            self.pos += buf.len() as u64;
            self.writes += 1;
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Seek for StrictDisk {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            if let SeekFrom::Start(p) = pos {
                self.pos = p;
            }
            Ok(self.pos)
        }
    }

    impl Target for StrictDisk {}

    fn disk(len: usize) -> StrictDisk {
        StrictDisk {
            data: (0..len).map(|i| (i % 97) as u8).collect(),
            pos: 0,
            sector: 512,
            writes: 0,
        }
    }

    #[test]
    fn random_access_matches_a_plain_buffer() {
        let len = 40 * BLOCK + 8 * 512;
        let base = 4 * BLOCK as u64;
        let mut dev = disk(base as usize + len);
        let mut model = dev.data.clone();
        let mut seed = 12345u64;
        let mut rnd = |m: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % m
        };
        {
            let mut c = BlockCache::new(&mut dev, base, len as u64, 512);
            for round in 0..3000 {
                let pos = rnd(len as u64 - 5000);
                let n = 1 + rnd(4999) as usize;
                c.seek(SeekFrom::Start(pos)).unwrap();
                if round % 3 == 0 {
                    let mut got = vec![0u8; n];
                    c.read_exact(&mut got).unwrap();
                    let at = base as usize + pos as usize;
                    assert_eq!(got, &model[at..at + n], "read at {pos}");
                } else {
                    let data: Vec<u8> = (0..n).map(|_| rnd(256) as u8).collect();
                    c.write_all(&data).unwrap();
                    let at = base as usize + pos as usize;
                    model[at..at + n].copy_from_slice(&data);
                }
            }
            c.flush().unwrap();
        }
        assert!(dev.data == model, "device differs after flush");
    }

    #[test]
    fn sequential_writes_go_out_as_whole_blocks() {
        let mut dev = disk(8 * BLOCK);
        {
            let mut c = BlockCache::new(&mut dev, 0, 8 * BLOCK as u64, 512);
            let chunk = vec![0xAB; 32 * 1024];
            for _ in 0..(8 * BLOCK / chunk.len()) {
                c.write_all(&chunk).unwrap();
            }
            c.flush().unwrap();
        }
        assert!(dev.data.iter().all(|&b| b == 0xAB));
        assert_eq!(dev.writes, 8, "one write per block, no read-modify-write");
    }

    #[test]
    fn works_over_a_cursor() {
        let mut dev = Cursor::new(vec![0u8; 3 * BLOCK]);
        let mut c = BlockCache::new(&mut dev, 0, 3 * BLOCK as u64, 512);
        assert_eq!(c.seek(SeekFrom::End(0)).unwrap(), 3 * BLOCK as u64);
        c.seek(SeekFrom::Start(10)).unwrap();
        c.write_all(b"hello").unwrap();
        c.flush().unwrap();
        assert_eq!(&dev.get_ref()[10..15], b"hello");
    }
}
