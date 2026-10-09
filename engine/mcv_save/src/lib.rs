//! Save format: `level.meta` + region files with RLE-compressed chunk voxels.
//!
//! Region file layout (all little-endian):
//! - header: 512 entries of { u32 byte_offset, u32 byte_len }, offset 0 =
//!   unused
//! - body records, appended: { u16 chunk_local, u8 version, u32 rle_len,
//!   rle bytes }
//!
//! RLE (u16 block ids, v2+): sequence of (count u8, id u16 LE) for runs
//! 1..=255; count 0 escapes to (0, len_lo, len_hi, id_lo, id_hi) supporting
//! runs up to 65535. `id` is a little-endian u16 `BlockId`.
//!
//! v1 (u8 ids) is *not* readable: development format, old regions are
//! discarded and `load_chunk` reports a version error.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const REGION_CHUNKS: usize = 16 * 16;
pub const HEADER_SIZE: usize = REGION_CHUNKS * 8;
const RECORD_HEADER: usize = 2 + 1 + 4;
/// Region record format version. v2 = u16 block ids (RLE widened with the
/// block-id u8 -> u16 migration); v1 files are rejected, not migrated.
pub const CHUNK_VERSION: u8 = 2;

#[derive(Debug)]
pub enum SaveError {
    Io(io::Error),
    Corrupt(&'static str),
}

impl From<io::Error> for SaveError {
    fn from(e: io::Error) -> Self {
        SaveError::Io(e)
    }
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Io(e) => write!(f, "io error: {e}"),
            SaveError::Corrupt(m) => write!(f, "corrupt save: {m}"),
        }
    }
}

impl std::error::Error for SaveError {}

pub fn rle_encode(data: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 16);
    let mut i = 0;
    while i < data.len() {
        let id = data[i];
        let mut run = 1usize;
        while i + run < data.len() && data[i + run] == id && run < 65535 {
            run += 1;
        }
        if run < 256 {
            out.push(run as u8);
            out.extend_from_slice(&id.to_le_bytes());
        } else {
            out.push(0);
            out.push((run & 0xFF) as u8);
            out.push((run >> 8) as u8);
            out.extend_from_slice(&id.to_le_bytes());
        }
        i += run;
    }
    out
}

pub fn rle_decode(data: &[u8], out: &mut [u16]) -> Result<(), SaveError> {
    let mut i = 0;
    let mut pos = 0;
    while i < data.len() {
        let count = data[i];
        if count == 0 {
            if i + 4 >= data.len() {
                return Err(SaveError::Corrupt("truncated escape run"));
            }
            let run = data[i + 1] as usize | ((data[i + 2] as usize) << 8);
            let id = u16::from_le_bytes([data[i + 3], data[i + 4]]);
            if pos + run > out.len() {
                return Err(SaveError::Corrupt("escape run overruns output"));
            }
            out[pos..pos + run].fill(id);
            pos += run;
            i += 5;
        } else {
            if i + 2 >= data.len() {
                return Err(SaveError::Corrupt("truncated run"));
            }
            let id = u16::from_le_bytes([data[i + 1], data[i + 2]]);
            if pos + count as usize > out.len() {
                return Err(SaveError::Corrupt("run overruns output"));
            }
            out[pos..pos + count as usize].fill(id);
            pos += count as usize;
            i += 3;
        }
    }
    if pos != out.len() {
        return Err(SaveError::Corrupt("decoded length mismatch"));
    }
    Ok(())
}

/// `level.meta` — world identity + player state.
pub struct LevelMeta {
    pub seed: u64,
    pub name: String,
    /// Absolute day time in ticks (24000 ticks per day, MC convention).
    pub day_time: u64,
    /// 0 = survival, 1 = creative, 2 = hardcore.
    pub mode: u8,
    pub player: Option<PlayerMeta>,
}

pub struct PlayerMeta {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub flying: bool,
    pub sel_slot: u8,
    /// v3 快捷栏:(item, count, damage) × ≤9,槽序即下标;v1/v2 读为空。
    pub hotbar: Vec<(u16, u8, u16)>,
}

impl LevelMeta {
    pub const MAGIC: [u8; 4] = *b"MCV1";

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        out.extend_from_slice(&Self::MAGIC);
        out.extend_from_slice(&3u16.to_le_bytes()); // format version
        out.extend_from_slice(&self.seed.to_le_bytes());
        out.extend_from_slice(&self.day_time.to_le_bytes());
        let name = self.name.as_bytes();
        out.push(name.len().min(255) as u8);
        out.extend_from_slice(&name[..name.len().min(255)]);
        match &self.player {
            None => out.push(0),
            Some(p) => {
                out.push(1);
                out.extend_from_slice(&p.x.to_le_bytes());
                out.extend_from_slice(&p.y.to_le_bytes());
                out.extend_from_slice(&p.z.to_le_bytes());
                out.extend_from_slice(&p.yaw.to_le_bytes());
                out.extend_from_slice(&p.pitch.to_le_bytes());
                out.push(u8::from(p.flying));
                out.push(p.sel_slot);
                // v3 快捷栏(模式字节保持在末尾,兼容旧解码位置约定)。
                let hb = &p.hotbar[..p.hotbar.len().min(9)];
                out.push(hb.len() as u8);
                for (item, count, damage) in hb {
                    out.extend_from_slice(&item.to_le_bytes());
                    out.push(*count);
                    out.extend_from_slice(&damage.to_le_bytes());
                }
            }
        }
        out.push(self.mode);
        out
    }

    pub fn decode(data: &[u8]) -> Result<Self, SaveError> {
        if data.len() < 4 + 2 + 8 + 8 || data[..4] != Self::MAGIC {
            return Err(SaveError::Corrupt("bad magic"));
        }
        let ver = u16::from_le_bytes([data[4], data[5]]);
        if ver > 3 {
            return Err(SaveError::Corrupt("unsupported meta version"));
        }
        let seed = u64::from_le_bytes(data[6..14].try_into().unwrap());
        let day_time = u64::from_le_bytes(data[14..22].try_into().unwrap());
        let mut i = 22;
        let name_len = data[i] as usize;
        i += 1;
        if i + name_len > data.len() {
            return Err(SaveError::Corrupt("truncated name"));
        }
        let name = String::from_utf8_lossy(&data[i..i + name_len]).into_owned();
        i += name_len;
        let player = match data.get(i) {
            Some(0) | None => None,
            Some(1) => {
                i += 1;
                let need = 4 * 5 + 2;
                if i + need > data.len() {
                    return Err(SaveError::Corrupt("truncated player"));
                }
                let f32_at =
                    |o: usize| f32::from_le_bytes(data[i + o..i + o + 4].try_into().unwrap());
                let (x, y, z, yaw, pitch) = (f32_at(0), f32_at(4), f32_at(8), f32_at(12), f32_at(16));
                let (flying, sel_slot) = (data[i + 20] != 0, data[i + 21]);
                i += 4 * 5 + 2;
                let mut hotbar = Vec::new();
                if ver >= 3 {
                    let n = *data.get(i).ok_or(SaveError::Corrupt("truncated hotbar"))? as usize;
                    let n = n.min(9);
                    i += 1;
                    if i + n * 5 > data.len() {
                        return Err(SaveError::Corrupt("truncated hotbar"));
                    }
                    for k in 0..n {
                        let o = i + k * 5;
                        hotbar.push((
                            u16::from_le_bytes([data[o], data[o + 1]]),
                            data[o + 2],
                            u16::from_le_bytes([data[o + 3], data[o + 4]]),
                        ));
                    }
                }
                Some(PlayerMeta {
                    x,
                    y,
                    z,
                    yaw,
                    pitch,
                    flying,
                    sel_slot,
                    hotbar,
                })
            }
            Some(_) => return Err(SaveError::Corrupt("bad player flag")),
        };
        // v2 appends the game mode byte; v1 files are survival.
        let mode = if ver >= 2 {
            let m = data.last().copied().unwrap_or(0);
            if m > 2 {
                return Err(SaveError::Corrupt("bad game mode"));
            }
            m
        } else {
            0
        };
        Ok(Self {
            seed,
            name,
            day_time,
            mode,
            player,
        })
    }
}

/// One `r.<rx>.<rz>.mcrv` file covering a 16x16 chunk region.
pub struct RegionFile {
    path: PathBuf,
    file: fs::File,
}

fn region_path(dir: &Path, rx: i32, rz: i32) -> PathBuf {
    let region_dir = dir.join("region");
    region_dir.join(format!("r.{rx}.{rz}.mcrv"))
}

impl RegionFile {
    pub fn open(dir: &Path, rx: i32, rz: i32) -> io::Result<Self> {
        let path = region_path(dir, rx, rz);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        if file.metadata()?.len() < HEADER_SIZE as u64 {
            file.set_len(HEADER_SIZE as u64)?;
        }
        Ok(Self { path, file })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read_header(&mut self) -> io::Result<[u8; HEADER_SIZE]> {
        let mut header = [0u8; HEADER_SIZE];
        self.file.seek(SeekFrom::Start(0))?;
        self.file.read_exact(&mut header)?;
        Ok(header)
    }

    fn write_header(&mut self, header: &[u8; HEADER_SIZE]) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(header)
    }

    fn slot(&self, local: u16) -> usize {
        (local as usize) * 8
    }

    /// Saves one chunk (local index 0..255, (z<<4)|x within the region).
    /// Reuses the old slot when the compressed record still fits, else
    /// appends. Writes tmp + rename is unnecessary here (record granularity).
    pub fn save_chunk(&mut self, local: u16, voxels: &[u16]) -> io::Result<()> {
        debug_assert!(voxels.len() == mcv_core::CHUNK_VOL);
        let rle = rle_encode(voxels);
        let record_len = RECORD_HEADER + rle.len();
        let header = self.read_header()?;

        let s = self.slot(local);
        let old_off = u32::from_le_bytes(header[s..s + 4].try_into().unwrap());
        let old_len = u32::from_le_bytes(header[s + 4..s + 8].try_into().unwrap());

        let (off, in_place) = if old_off != 0 && (record_len as u32) <= old_len {
            (old_off, true)
        } else {
            (self.file.seek(SeekFrom::End(0))? as u32, false)
        };

        let mut record = Vec::with_capacity(record_len);
        record.extend_from_slice(&local.to_le_bytes());
        record.push(CHUNK_VERSION);
        record.extend_from_slice(&(rle.len() as u32).to_le_bytes());
        record.extend_from_slice(&rle);

        self.file.seek(SeekFrom::Start(off as u64))?;
        self.file.write_all(&record)?;
        self.file.flush()?;

        let mut header = header;
        header[s..s + 4].copy_from_slice(&off.to_le_bytes());
        header[s + 4..s + 8].copy_from_slice(&(record_len as u32).to_le_bytes());
        self.write_header(&header)?;
        let _ = in_place;
        Ok(())
    }

    pub fn load_chunk(&mut self, local: u16, out: &mut [u16]) -> Result<(), SaveError> {
        debug_assert!(out.len() == mcv_core::CHUNK_VOL);
        let header = self.read_header()?;
        let s = self.slot(local);
        let off = u32::from_le_bytes(header[s..s + 4].try_into().unwrap());
        let len = u32::from_le_bytes(header[s + 4..s + 8].try_into().unwrap());
        if off == 0 {
            return Err(SaveError::Corrupt("chunk not saved"));
        }
        self.file.seek(SeekFrom::Start(off as u64))?;
        let mut record = vec![0u8; len as usize];
        self.file.read_exact(&mut record)?;
        if record.len() < RECORD_HEADER {
            return Err(SaveError::Corrupt("short record"));
        }
        let stored_local = u16::from_le_bytes([record[0], record[1]]);
        if stored_local != local {
            return Err(SaveError::Corrupt("chunk local mismatch"));
        }
        let version = record[2];
        if version != CHUNK_VERSION {
            // Dev-format break: u8-id (v1) regions are not migrated.
            return Err(SaveError::Corrupt("unsupported region chunk version"));
        }
        let rle_len = u32::from_le_bytes(record[3..7].try_into().unwrap()) as usize;
        if record.len() < RECORD_HEADER + rle_len {
            return Err(SaveError::Corrupt("short rle"));
        }
        rle_decode(&record[RECORD_HEADER..RECORD_HEADER + rle_len], out)
    }

    pub fn has_chunk(&mut self, local: u16) -> io::Result<bool> {
        let header = self.read_header()?;
        let s = self.slot(local);
        Ok(u32::from_le_bytes(header[s..s + 4].try_into().unwrap()) != 0)
    }
}

pub fn chunk_region(cx: i32, cz: i32) -> (i32, i32) {
    (cx.div_euclid(16), cz.div_euclid(16))
}

pub fn chunk_local(cx: i32, cz: i32) -> u16 {
    ((cz.rem_euclid(16) as u16) << 4) | (cx.rem_euclid(16) as u16)
}
