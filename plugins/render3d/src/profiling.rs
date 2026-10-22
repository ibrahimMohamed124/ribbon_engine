//! Asynchronous GPU timestamp readback for shadow and main mesh rendering.
//!
//! The profiler never waits for the GPU. It uses a small readback-buffer ring, reports completed
//! samples on a later frame, and skips a sample when every ring slot is still busy.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Mutex;

const TIMESTAMP_BYTES: u64 = 8;
const TIMESTAMP_COUNT: u32 = 3;
const READBACK_SLOTS: usize = 4;
const BUFFER_BYTES: u64 = TIMESTAMP_BYTES * TIMESTAMP_COUNT as u64;
type TimestampMapResult = Result<(), wgpu::BufferAsyncError>;
type TimestampReceiver = Mutex<Receiver<TimestampMapResult>>;

struct TimestampSlot {
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    // Receiver itself is not Sync; protect polling to keep the render pass Sync.
    receiver: Option<TimestampReceiver>,
    reserved: bool,
    sequence: u64,
}

/// Snapshot of timestamp profiler state for diagnostics/UI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuTimingSnapshot {
    /// Whether the device supports timestamp queries.
    pub supported: bool,
    /// Most recent completed directional/spot-shadow plus mesh GPU duration in nanoseconds.
    pub latest_duration_ns: Option<u64>,
    /// Most recent completed main color/depth mesh pass GPU duration in nanoseconds.
    pub latest_mesh_pass_ns: Option<u64>,
    /// Monotonic profiler sequence number for the latest completed sample.
    pub latest_sequence: Option<u64>,
    /// Number of successfully completed timestamp samples.
    pub completed_samples: u64,
    /// Number of attempted frames skipped because every readback slot was busy.
    pub skipped_samples: u64,
    /// Bytes used by the timestamp resolve/readback buffers (query-set storage is driver-owned).
    pub buffer_bytes_estimate: u64,
}

pub(crate) struct GpuTimestampProfiler {
    query_set: wgpu::QuerySet,
    slots: Vec<TimestampSlot>,
    next_slot: usize,
    sequence: u64,
    latest_duration_ns: Option<u64>,
    latest_mesh_pass_ns: Option<u64>,
    latest_sequence: Option<u64>,
    completed_samples: u64,
    skipped_samples: u64,
}

impl GpuTimestampProfiler {
    pub(crate) fn new(device: &wgpu::Device) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("render3d mesh pass timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: TIMESTAMP_COUNT,
        });
        let slots = (0..READBACK_SLOTS)
            .map(|_| TimestampSlot {
                resolve: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("render3d timestamp resolve buffer"),
                    size: BUFFER_BYTES,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }),
                readback: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("render3d timestamp readback buffer"),
                    size: BUFFER_BYTES,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                receiver: None,
                reserved: false,
                sequence: 0,
            })
            .collect();
        Some(Self {
            query_set,
            slots,
            next_slot: 0,
            sequence: 0,
            latest_duration_ns: None,
            latest_mesh_pass_ns: None,
            latest_sequence: None,
            completed_samples: 0,
            skipped_samples: 0,
        })
    }

    /// Reserve a result slot for the upcoming mesh submission. A slot remains occupied until
    /// asynchronous mapping has completed and its buffer has been unmapped.
    pub(crate) fn reserve_slot(&mut self) -> Option<usize> {
        let slot_index = (0..self.slots.len())
            .map(|offset| (self.next_slot + offset) % self.slots.len())
            .find(|index| {
                let slot = &self.slots[*index];
                !slot.reserved && slot.receiver.is_none()
            });
        let Some(slot_index) = slot_index else {
            self.skipped_samples = self.skipped_samples.saturating_add(1);
            return None;
        };
        self.next_slot = (slot_index + 1) % self.slots.len();
        self.sequence = self.sequence.saturating_add(1);
        let slot = &mut self.slots[slot_index];
        slot.reserved = true;
        slot.sequence = self.sequence;
        Some(slot_index)
    }

    pub(crate) fn query_set(&self) -> &wgpu::QuerySet {
        &self.query_set
    }

    /// Append query resolve/copy operations after the measured rendering passes.
    pub(crate) fn encode_resolve(&self, encoder: &mut wgpu::CommandEncoder, slot_index: usize) {
        let slot = &self.slots[slot_index];
        encoder.resolve_query_set(&self.query_set, 0..TIMESTAMP_COUNT, &slot.resolve, 0);
        encoder.copy_buffer_to_buffer(&slot.resolve, 0, &slot.readback, 0, BUFFER_BYTES);
    }

    /// Begin the asynchronous readback after the command buffer has been submitted.
    pub(crate) fn submitted(&mut self, slot_index: usize) {
        let slot = &mut self.slots[slot_index];
        let (sender, receiver) = mpsc::channel();
        slot.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        slot.receiver = Some(Mutex::new(receiver));
        slot.reserved = false;
    }

    /// Poll pending mappings without waiting for unfinished GPU work.
    pub(crate) fn poll(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let _ = device.poll(wgpu::PollType::Poll);
        for slot in &mut self.slots {
            let state = slot.receiver.as_ref().map(|receiver| {
                receiver
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .try_recv()
            });
            match state {
                Some(Ok(Ok(()))) => {
                    let mapped = slot.readback.slice(..).get_mapped_range();
                    if mapped.len() >= BUFFER_BYTES as usize {
                        let shadow_start = u64::from_le_bytes(
                            mapped[0..8]
                                .try_into()
                                .expect("timestamp readback has an eight-byte shadow start value"),
                        );
                        let mesh_start = u64::from_le_bytes(
                            mapped[8..16]
                                .try_into()
                                .expect("timestamp readback has an eight-byte mesh start value"),
                        );
                        let mesh_end = u64::from_le_bytes(
                            mapped[16..24]
                                .try_into()
                                .expect("timestamp readback has an eight-byte mesh end value"),
                        );
                        let period_ns = f64::from(queue.get_timestamp_period());
                        let total_ns = elapsed_ns(shadow_start, mesh_end, period_ns);
                        let mesh_ns = elapsed_ns(mesh_start, mesh_end, period_ns);
                        if let Some(duration) = total_ns {
                            let is_newest = self
                                .latest_sequence
                                .is_none_or(|latest| slot.sequence >= latest);
                            if is_newest {
                                self.latest_duration_ns = Some(duration);
                                self.latest_mesh_pass_ns = mesh_ns;
                                self.latest_sequence = Some(slot.sequence);
                            }
                            self.completed_samples = self.completed_samples.saturating_add(1);
                        }
                    }
                    drop(mapped);
                    slot.readback.unmap();
                    slot.receiver = None;
                }
                Some(Ok(Err(_))) | Some(Err(TryRecvError::Disconnected)) => {
                    slot.receiver = None;
                }
                Some(Err(TryRecvError::Empty)) | None => {}
            }
        }
    }

    pub(crate) fn snapshot(&self) -> GpuTimingSnapshot {
        GpuTimingSnapshot {
            supported: true,
            latest_duration_ns: self.latest_duration_ns,
            latest_mesh_pass_ns: self.latest_mesh_pass_ns,
            latest_sequence: self.latest_sequence,
            completed_samples: self.completed_samples,
            skipped_samples: self.skipped_samples,
            buffer_bytes_estimate: (self.slots.len() as u64).saturating_mul(BUFFER_BYTES * 2),
        }
    }
}

fn elapsed_ns(start: u64, end: u64, period_ns: f64) -> Option<u64> {
    let ticks = end.wrapping_sub(start);
    if ticks > i64::MAX as u64 || !period_ns.is_finite() || period_ns <= 0.0 {
        return None;
    }
    let nanoseconds = (ticks as f64 * period_ns).round();
    (nanoseconds.is_finite() && nanoseconds >= 0.0)
        .then_some(nanoseconds.min(u64::MAX as f64) as u64)
}
