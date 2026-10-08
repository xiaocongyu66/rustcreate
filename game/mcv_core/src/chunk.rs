//! Chunk storage and the chunk pipeline state machine.
//!
//! Stages advance one way; corrections only flip dirty bits.
//! Empty -> TerrainReady -> LightLocalReady -> Lit -> MeshReady -> Uploaded.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::RwLock;

use crate::{BlockId, ChunkPos, CHUNK_VOL};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Stage {
    Empty = 0,
    TerrainReady = 1,
    LightLocalReady = 2,
    Lit = 3,
    MeshReady = 4,
    Uploaded = 5,
}

pub mod dirty {
    pub const LIGHT: u8 = 1;
    pub const MESH: u8 = 2;
    pub const SAVE: u8 = 4;
}

/// Shared per-chunk state. Workers commit owned buffers; readers lock
/// narrowly. The stage counter is monotonic (see [`Stage`]).
pub struct ChunkHandle {
    pub pos: ChunkPos,
    stage: AtomicU8,
    dirty: AtomicU8,
    pub voxels: RwLock<Box<[BlockId; CHUNK_VOL]>>,
    pub heightmap: RwLock<Box<[u8; 256]>>,
    pub light: RwLock<Box<[u8; CHUNK_VOL]>>,
}

impl ChunkHandle {
    pub fn new(pos: ChunkPos) -> Self {
        Self {
            pos,
            stage: AtomicU8::new(Stage::Empty as u8),
            dirty: AtomicU8::new(0),
            voxels: RwLock::new(Box::new([BlockId(0); CHUNK_VOL])),
            heightmap: RwLock::new(Box::new([0; 256])),
            light: RwLock::new(Box::new([0; CHUNK_VOL])),
        }
    }

    pub fn stage(&self) -> Stage {
        match self.stage.load(Ordering::Acquire) {
            1 => Stage::TerrainReady,
            2 => Stage::LightLocalReady,
            3 => Stage::Lit,
            4 => Stage::MeshReady,
            5 => Stage::Uploaded,
            _ => Stage::Empty,
        }
    }

    /// Sets the stage if it is an advance (never regresses).
    pub fn advance_to(&self, target: Stage) {
        let t = target as u8;
        let mut cur = self.stage.load(Ordering::Acquire);
        while cur < t {
            match self
                .stage
                .compare_exchange_weak(cur, t, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => break,
                Err(actual) => cur = actual,
            }
        }
    }

    pub fn reset(&self) {
        self.stage.store(Stage::Empty as u8, Ordering::Release);
        self.dirty.store(0, Ordering::Release);
    }

    pub fn dirty(&self) -> u8 {
        self.dirty.load(Ordering::Acquire)
    }

    pub fn mark_dirty(&self, bits: u8) {
        self.dirty.fetch_or(bits, Ordering::AcqRel);
    }

    pub fn clear_dirty(&self, bits: u8) {
        self.dirty.fetch_and(!bits, Ordering::AcqRel);
    }
}
