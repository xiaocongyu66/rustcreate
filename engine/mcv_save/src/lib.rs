//! Save format: `level.meta` + region files with RLE-compressed chunk voxels.
//!
//! `level.meta` format version history: v3 adds the hotbar, v4 the 27-slot
//! main inventory, v5 the survival stats (health/hunger/saturation/exhaustion/
//! air_supply/difficulty, appended before the trailing mode byte). Decoders
//! accept ≤ their own version and default the missing fields.
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
    /// v4 主背包:(item, count, damage) × ≤27,槽序即下标;v1–v3 读为空。
    pub main: Vec<(u16, u8, u16)>,
    /// v5 生命值(0..20);v1–v4 读默认 20.0。
    pub health: f32,
    /// v5 饥饿值(0..20,FoodData.foodLevel);v1–v4 读默认 20.0。
    pub hunger: f32,
    /// v5 饱和度(FoodData.saturationLevel);v1–v4 读默认 5.0(开局值,
    /// FoodConstants.java:6 START_SATURATION)。
    pub saturation: f32,
    /// v5 饥饿消耗(FoodData.exhaustionLevel);v1–v4 读默认 0.0。
    pub exhaustion: f32,
    /// v5 空气供给(Entity.airSupply,满值 300);v1–v4 读默认 300。
    pub air_supply: i32,
    /// v5 难度 id(Difficulty.java:28-30:0和平/1简单/2普通/3困难);
    /// v1–v4 读默认 2(普通,与运行时起步一致)。
    pub difficulty: u8,
}

/// (item, count, damage) 列表编解码(v3 快捷栏 / v4 主背包共用)。
fn encode_stacks(out: &mut Vec<u8>, stacks: &[(u16, u8, u16)]) {
    out.push(stacks.len().min(255) as u8);
    for (item, count, damage) in stacks.iter().take(255) {
        out.extend_from_slice(&item.to_le_bytes());
        out.push(*count);
        out.extend_from_slice(&damage.to_le_bytes());
    }
}

fn decode_stacks(data: &[u8], i: &mut usize, max: usize) -> Result<Vec<(u16, u8, u16)>, SaveError> {
    let n = *data.get(*i).ok_or(SaveError::Corrupt("truncated hotbar"))? as usize;
    let n = n.min(max);
    *i += 1;
    if *i + n * 5 > data.len() {
        return Err(SaveError::Corrupt("truncated hotbar"));
    }
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let o = *i + k * 5;
        out.push((
            u16::from_le_bytes([data[o], data[o + 1]]),
            data[o + 2],
            u16::from_le_bytes([data[o + 3], data[o + 4]]),
        ));
    }
    *i += n * 5;
    Ok(out)
}

impl LevelMeta {
    pub const MAGIC: [u8; 4] = *b"MCV1";

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        out.extend_from_slice(&Self::MAGIC);
        out.extend_from_slice(&5u16.to_le_bytes()); // format version
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
                // v3 快捷栏(模式字节保持在末尾,兼容旧解码位置约定);
                // v4 起快捷栏补齐 9 槽(空槽 (0,0,0) 占位,槽序即下标)并
                // 追加 27 槽主背包。
                let mut hb = p.hotbar.clone();
                hb.resize(9, (0, 0, 0));
                encode_stacks(&mut out, &hb[..9]);
                let mut m = p.main.clone();
                m.resize(27, (0, 0, 0));
                encode_stacks(&mut out, &m[..27]);
                // v5 生存数值(health/hunger/saturation/exhaustion/air/difficulty)
                // 追加在模式字节之前——mode 仍居末位,旧解码位置约定不变。
                out.extend_from_slice(&p.health.to_le_bytes());
                out.extend_from_slice(&p.hunger.to_le_bytes());
                out.extend_from_slice(&p.saturation.to_le_bytes());
                out.extend_from_slice(&p.exhaustion.to_le_bytes());
                out.extend_from_slice(&p.air_supply.to_le_bytes());
                out.push(p.difficulty);
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
        if ver > 5 {
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
                let (x, y, z, yaw, pitch) =
                    (f32_at(0), f32_at(4), f32_at(8), f32_at(12), f32_at(16));
                let (flying, sel_slot) = (data[i + 20] != 0, data[i + 21]);
                i += 4 * 5 + 2;
                // v3 快捷栏、v4 主背包、v5 生存数值;更旧的档读默认值。
                let hotbar = if ver >= 3 {
                    decode_stacks(data, &mut i, 9)?
                } else {
                    Vec::new()
                };
                let main = if ver >= 4 {
                    decode_stacks(data, &mut i, 27)?
                } else {
                    Vec::new()
                };
                // v5:4×f32(health/hunger/saturation/exhaustion) + i32(air)
                // + u8(difficulty) = 21 字节;v1–v4 读开局默认值。
                let (health, hunger, saturation, exhaustion, air_supply, difficulty) = if ver >= 5 {
                    const SURVIVAL_BYTES: usize = 4 * 4 + 4 + 1;
                    if i + SURVIVAL_BYTES > data.len() {
                        return Err(SaveError::Corrupt("truncated survival stats"));
                    }
                    let f32_at =
                        |o: usize| f32::from_le_bytes(data[i + o..i + o + 4].try_into().unwrap());
                    let air = i32::from_le_bytes(data[i + 16..i + 20].try_into().unwrap());
                    let difficulty = data[i + 20];
                    i += SURVIVAL_BYTES;
                    (f32_at(0), f32_at(4), f32_at(8), f32_at(12), air, difficulty)
                } else {
                    // 与 Player::default / FoodData 起步式一致(开局满血满饥饿、
                    // 饱和 5.0、空气满、难度普通)。
                    (20.0, 20.0, 5.0, 0.0, 300, 2)
                };
                Some(PlayerMeta {
                    x,
                    y,
                    z,
                    yaw,
                    pitch,
                    flying,
                    sel_slot,
                    hotbar,
                    main,
                    health,
                    hunger,
                    saturation,
                    exhaustion,
                    air_supply,
                    difficulty,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// v5 往返：写 → 读 → 逐字段相等（含生存数值与快捷栏/主背包）。
    #[test]
    fn v5_roundtrip_preserves_every_field() {
        let meta = LevelMeta {
            seed: 0xDEAD_BEEF_1234_5678,
            name: "测试世界".into(),
            day_time: 123_456,
            mode: 2,
            player: Some(PlayerMeta {
                x: 1.5,
                y: -3.25,
                z: 255.75,
                yaw: 179.5,
                pitch: -89.25,
                flying: true,
                sel_slot: 8,
                // 编码器按 9/27 槽全量落盘（空槽 (0,0,0) 占位）——用满表
                // 才能逐槽相等。
                hotbar: {
                    let mut v = vec![(0u16, 0u8, 0u16); 9];
                    v[0] = (7, 1, 3);
                    v[1] = (36, 12, 0);
                    v[8] = (5, 2, 1);
                    v
                },
                main: vec![(27, 64, 0); 27],
                health: 13.5,
                hunger: 17.0,
                saturation: 4.25,
                exhaustion: 39.75,
                air_supply: 217,
                difficulty: 3,
            }),
        };
        let bytes = meta.encode();
        let back = LevelMeta::decode(&bytes).expect("v5 档可读");
        assert_eq!(back.seed, meta.seed);
        assert_eq!(back.name, meta.name);
        assert_eq!(back.day_time, meta.day_time);
        assert_eq!(back.mode, meta.mode);
        let p = back.player.expect("player 在档");
        let q = meta.player.as_ref().unwrap();
        assert_eq!((p.x, p.y, p.z), (q.x, q.y, q.z));
        assert_eq!((p.yaw, p.pitch), (q.yaw, q.pitch));
        assert_eq!((p.flying, p.sel_slot), (q.flying, q.sel_slot));
        assert_eq!(p.hotbar, q.hotbar);
        assert_eq!(p.main, q.main);
        // 生存数值逐字段相等（本任务核心）。
        assert_eq!((p.health, q.health), (13.5, 13.5));
        assert_eq!((p.hunger, q.hunger), (17.0, 17.0));
        assert_eq!((p.saturation, q.saturation), (4.25, 4.25));
        assert_eq!((p.exhaustion, q.exhaustion), (39.75, 39.75));
        assert_eq!((p.air_supply, q.air_supply), (217, 217));
        assert_eq!((p.difficulty, q.difficulty), (3, 3));
    }

    /// 旧 v4 档（快捷栏 + 主背包，无生存数值）读入：生存六项给默认值，
    /// 其余字段原样保留——退出重进不因格式升级丢开局状态。
    #[test]
    fn v4_file_reads_with_survival_defaults() {
        // 手工拼一份 v4 字节（与 v4 编码器布局一致：生存数值尚未存在，
        // mode 居末位）。
        let mut out = Vec::new();
        out.extend_from_slice(b"MCV1");
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&42u64.to_le_bytes());
        out.extend_from_slice(&777u64.to_le_bytes());
        let name = b"world";
        out.push(name.len() as u8);
        out.extend_from_slice(name);
        out.push(1); // player flag
        for v in [8.5f32, 71.0, 8.5, 90.0, 0.0] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.push(0); // flying
        out.push(3); // sel_slot
        // 快捷栏 2 槽。
        out.push(2);
        for (item, count, damage) in [(7u16, 1u8, 0u16), (36, 5, 2)] {
            out.extend_from_slice(&item.to_le_bytes());
            out.push(count);
            out.extend_from_slice(&damage.to_le_bytes());
        }
        // 主背包空（27 槽占位）。
        out.push(0);
        out.push(2); // mode（v2 起居末位）
        let back = LevelMeta::decode(&out).expect("v4 档仍可读");
        assert_eq!(back.seed, 42);
        assert_eq!(back.day_time, 777);
        assert_eq!(back.mode, 2);
        let p = back.player.expect("player 在档");
        assert_eq!((p.x, p.y, p.z), (8.5, 71.0, 8.5));
        assert_eq!(p.sel_slot, 3);
        assert_eq!(p.hotbar, vec![(7, 1, 0), (36, 5, 2)]);
        assert!(p.main.is_empty());
        // v4 无生存数值 → 开局默认（Player::default 等价）。
        assert_eq!(p.health, 20.0);
        assert_eq!(p.hunger, 20.0);
        assert_eq!(p.saturation, 5.0);
        assert_eq!(p.exhaustion, 0.0);
        assert_eq!(p.air_supply, 300);
        assert_eq!(p.difficulty, 2);
    }

    /// 超前版本拒读（新档喂给旧解码器 → 明确报错，不静默错读）。
    #[test]
    fn future_version_rejected() {
        let mut out = Vec::new();
        out.extend_from_slice(b"MCV1");
        out.extend_from_slice(&6u16.to_le_bytes());
        out.extend_from_slice(&[0u8; 24]);
        assert!(LevelMeta::decode(&out).is_err());
    }
}
